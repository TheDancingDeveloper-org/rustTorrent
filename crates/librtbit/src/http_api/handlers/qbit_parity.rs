//! qBittorrent WebUI API parity checker.
//!
//! `qbit_parity_spec.json` is a machine-readable inventory of the upstream
//! qBittorrent WebUI API v2 surface (extracted from the qBittorrent sources),
//! annotated with the parity status of each endpoint in our compat layer
//! (`qbit/mod.rs`). These tests keep the spec and the actual router from
//! drifting apart:
//!
//! - every endpoint marked `full` or `partial` must be routed;
//! - every endpoint marked `missing` or `out_of_scope` must return 404;
//! - every route literal in `qbit/mod.rs` must be tracked in the spec;
//! - every routed endpoint declares a `response` (content kind plus, for JSON,
//!   the documented upstream field set and types), and the router's live
//!   output — probed against a real in-process torrent — must match it.
//!
//! A JSON `schema` is written in a compact form: a type name (`string`,
//! `integer`, `number`, `boolean`, `any`), `[elem]` for an array, `{"*": v}`
//! for a map with arbitrary keys, or `{"field": type, ...}` for an object
//! whose listed fields must all be present. Extra fields are allowed. Upstream
//! fields we do not emit yet go in `missing_fields` (dotted paths, `*` for an
//! array element or map value); a listed field that starts appearing fails the
//! test so the list cannot go stale.
//!
//! When implementing a new compat endpoint, flip its `status` in the spec, add
//! its `response`, drop a note about any semantic gaps, and extend the
//! lifecycle test in this module if the endpoint changes torrent state. To refresh the upstream
//! inventory against a newer qBittorrent checkout, run
//! `scripts/refresh-qbit-parity-spec.py <path-to-qbittorrent-checkout>`.

use std::{collections::HashSet, net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

use http::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt;

use crate::{
    AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, ListenerMode, Session,
    SessionOptions, create_torrent,
    http_api::{HttpApi, HttpApiOptions},
    listen::ListenerOptions,
    spawn_utils::BlockingSpawner,
};

const SPEC_JSON: &str = include_str!("qbit_parity_spec.json");
const COMPAT_SOURCE: &str = include_str!("qbit/mod.rs");

#[derive(Deserialize)]
struct Spec {
    upstream: UpstreamInfo,
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct UpstreamInfo {
    webapi_version: String,
}

#[derive(Deserialize)]
struct Endpoint {
    /// `controller/action`, e.g. `torrents/info`.
    endpoint: String,
    method: String,
    status: Status,
    /// False for legacy endpoints we serve that upstream has since removed.
    #[serde(default = "default_true")]
    upstream: bool,
    /// Response contract; required for routed entries.
    response: Option<ResponseSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseSpec {
    content: Content,
    /// Expected JSON shape (only for `content: json`).
    schema: Option<serde_json::Value>,
    /// Upstream fields not emitted yet, as dotted paths.
    #[serde(default)]
    missing_fields: Vec<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum Content {
    /// JSON body validated against `schema`.
    Json,
    /// Plain-text body (`Ok.`, version strings, bare numbers).
    Text,
    /// Raw .torrent bytes.
    Bittorrent,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Routed, semantics match qBittorrent closely enough for real clients.
    Full,
    /// Routed, but with stubbed fields or semantic gaps (see notes).
    Partial,
    /// Not routed; candidate for future parity work.
    Missing,
    /// Not routed by explicit decision.
    OutOfScope,
}

impl Status {
    fn is_routed(self) -> bool {
        matches!(self, Status::Full | Status::Partial)
    }
}

fn parse_spec() -> Spec {
    serde_json::from_str(SPEC_JSON).expect("qbit_parity_spec.json must be valid JSON")
}

pub(super) async fn make_router() -> (axum::Router, Arc<Session>, tempfile::TempDir) {
    make_router_with(HttpApiOptions {
        allow_create: true,
        ..Default::default()
    })
    .await
}

async fn make_router_with(opts: HttpApiOptions) -> (axum::Router, Arc<Session>, tempfile::TempDir) {
    let output = tempfile::TempDir::with_prefix("qbit_parity").unwrap();
    let session = Session::new_with_opts(
        output.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let api = Api::new(
        session.clone(),
        None,
        #[cfg(feature = "tracing-subscriber-utils")]
        None,
    );
    let api_state = Arc::new(HttpApi::new(api, Some(opts)));
    (super::qbit::make_qbit_router(api_state), session, output)
}

#[test]
fn spec_is_well_formed() {
    let spec = parse_spec();
    assert!(!spec.upstream.webapi_version.is_empty());
    let mut seen = HashSet::new();
    for ep in &spec.endpoints {
        assert!(
            seen.insert(ep.endpoint.as_str()),
            "duplicate spec entry: {}",
            ep.endpoint
        );
        assert!(
            matches!(ep.method.as_str(), "GET" | "POST"),
            "{}: unexpected method {}",
            ep.endpoint,
            ep.method
        );
        assert!(
            ep.endpoint.split('/').count() == 2,
            "{}: endpoint must be controller/action",
            ep.endpoint
        );
        match (&ep.response, ep.status.is_routed()) {
            (None, true) => panic!("{}: routed entries must declare a response", ep.endpoint),
            (Some(_), false) => panic!(
                "{}: unrouted entries must not declare a response",
                ep.endpoint
            ),
            (Some(r), true) => {
                assert_eq!(
                    r.schema.is_some(),
                    r.content == Content::Json,
                    "{}: a schema is required for (and only for) json responses",
                    ep.endpoint
                );
                if let Some(schema) = &r.schema {
                    check_schema_syntax(schema)
                        .unwrap_or_else(|e| panic!("{}: bad schema: {e}", ep.endpoint));
                }
                assert!(
                    r.missing_fields.is_empty() || r.schema.is_some(),
                    "{}: missing_fields needs a schema",
                    ep.endpoint
                );
            }
            (None, false) => {}
        }
        if !ep.upstream {
            assert!(
                ep.status.is_routed(),
                "{}: non-upstream (legacy) entries only make sense if we route them",
                ep.endpoint
            );
        }
    }
}

/// Probes every spec endpoint against the real compat router and fails on any
/// mismatch between the declared parity status and what is actually routed.
#[tokio::test]
async fn spec_matches_router() {
    let spec = parse_spec();
    let (router, _session, _output) = make_router().await;

    let mut violations = Vec::new();
    for ep in &spec.endpoints {
        let request = http::Request::builder()
            .method(ep.method.as_str())
            .uri(format!("/{}", ep.endpoint))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::empty())
            .unwrap();
        let status = router.clone().oneshot(request).await.unwrap().status();
        // Anything but 404/405 (including 400 for probes lacking required
        // params) proves the route exists with the expected method.
        let routed = status != StatusCode::NOT_FOUND && status != StatusCode::METHOD_NOT_ALLOWED;
        if routed != ep.status.is_routed() {
            violations.push(format!(
                "{} {} is marked {:?} in qbit_parity_spec.json but the router returned {status}",
                ep.method, ep.endpoint, ep.status
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "qbit compat router and qbit_parity_spec.json disagree; \
         update the spec status (or the router) for:\n{}",
        violations.join("\n")
    );
}

/// Every routed endpoint outside `auth/*` must sit behind the SID-cookie
/// layer. A sub-router nested outside `protected_router` would otherwise ship
/// unauthenticated without any other test noticing.
#[tokio::test]
async fn every_non_auth_route_requires_a_session() {
    let spec = parse_spec();
    let (router, _session, _output) = make_router_with(HttpApiOptions {
        allow_create: true,
        basic_auth: Some(("admin".into(), "secret".into())),
        ..Default::default()
    })
    .await;

    let mut unprotected = Vec::new();
    let mut probed = 0;
    for ep in spec.endpoints.iter().filter(|ep| ep.status.is_routed()) {
        let request = http::Request::builder()
            .method(ep.method.as_str())
            .uri(format!("/{}", ep.endpoint))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rejected = status == StatusCode::FORBIDDEN && &body[..] == b"Not authenticated";
        if ep.endpoint.starts_with("auth/") {
            assert!(
                !rejected,
                "{} {} must be reachable without a session",
                ep.method, ep.endpoint
            );
            continue;
        }
        probed += 1;
        if !rejected {
            unprotected.push(format!("{} {} -> {status}", ep.method, ep.endpoint));
        }
    }

    assert!(probed > 0, "no non-auth routes probed");
    assert!(
        unprotected.is_empty(),
        "routes reachable without a SID cookie (nest them inside protected_router):\n{}",
        unprotected.join("\n")
    );
}

/// Every `.route("...")` literal in qbit/mod.rs must correspond to a spec
/// entry marked as routed, so new compat routes can't land untracked.
#[test]
fn every_compat_route_is_tracked_in_spec() {
    let spec = parse_spec();
    let routed_actions: HashSet<&str> = spec
        .endpoints
        .iter()
        .filter(|ep| ep.status.is_routed())
        .filter_map(|ep| ep.endpoint.split_once('/').map(|(_, action)| action))
        .collect();

    let mut untracked = Vec::new();
    for (idx, _) in COMPAT_SOURCE.match_indices(".route(\"/") {
        let start = idx + ".route(\"/".len();
        let action = COMPAT_SOURCE[start..]
            .split('"')
            .next()
            .expect("unterminated route literal");
        if !routed_actions.contains(action) {
            untracked.push(action);
        }
    }

    assert!(
        untracked.is_empty(),
        "routes in qbit/mod.rs with no full/partial entry in qbit_parity_spec.json: {untracked:?}"
    );
}

/// Not an assertion — prints the parity scoreboard (visible with
/// `cargo test -p swarmforge qbit_parity -- --nocapture`).
#[test]
fn parity_summary() {
    let spec = parse_spec();
    let upstream: Vec<&Endpoint> = spec.endpoints.iter().filter(|ep| ep.upstream).collect();
    let count = |status: Status| upstream.iter().filter(|ep| ep.status == status).count();
    let (full, partial, missing, oos) = (
        count(Status::Full),
        count(Status::Partial),
        count(Status::Missing),
        count(Status::OutOfScope),
    );
    let implemented = full + partial;
    eprintln!(
        "qBittorrent WebUI API v{} parity: {implemented}/{} endpoints routed \
         ({full} full, {partial} partial, {missing} missing, {oos} out of scope)",
        spec.upstream.webapi_version,
        upstream.len(),
    );
}

// ---------------------------------------------------------------------------
// Response shape conformance
// ---------------------------------------------------------------------------

const SCALAR_TYPES: [&str; 5] = ["string", "integer", "number", "boolean", "any"];

fn check_schema_syntax(schema: &Value) -> Result<(), String> {
    match schema {
        Value::String(name) if SCALAR_TYPES.contains(&name.as_str()) => Ok(()),
        Value::String(name) => Err(format!("unknown type {name:?}")),
        Value::Array(elem) if elem.len() == 1 => check_schema_syntax(&elem[0]),
        Value::Array(_) => Err("array schemas take exactly one element schema".into()),
        Value::Object(fields) if !fields.is_empty() => {
            if fields.contains_key("*") && fields.len() != 1 {
                return Err("a map schema (`*`) cannot also list named fields".into());
            }
            fields.values().try_for_each(check_schema_syntax)
        }
        _ => Err(format!("invalid schema node {schema}")),
    }
}

fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Validates `value` against a compact schema (see the module docs), pushing
/// one message per violation. `missing` holds the declared-absent paths;
/// every one that is confirmed absent is recorded in `confirmed_missing`.
fn validate_shape(
    schema: &Value,
    value: &Value,
    path: &str,
    missing: &HashSet<&str>,
    confirmed_missing: &mut HashSet<String>,
    errors: &mut Vec<String>,
) {
    let at = if path.is_empty() { "<root>" } else { path };
    match schema {
        Value::String(name) => {
            let ok = match name.as_str() {
                "string" => value.is_string(),
                "integer" => value.is_i64() || value.is_u64(),
                "number" => value.is_number(),
                "boolean" => value.is_boolean(),
                _ => true,
            };
            if !ok {
                errors.push(format!("{at}: expected {name}, got {value}"));
            }
        }
        Value::Array(elem) => match value.as_array() {
            Some(items) => {
                for item in items {
                    let child = join_path(path, "*");
                    validate_shape(&elem[0], item, &child, missing, confirmed_missing, errors);
                }
            }
            None => errors.push(format!("{at}: expected an array, got {value}")),
        },
        Value::Object(fields) => {
            let Some(object) = value.as_object() else {
                errors.push(format!("{at}: expected an object, got {value}"));
                return;
            };
            if let Some(elem) = fields.get("*") {
                let child = join_path(path, "*");
                for item in object.values() {
                    validate_shape(elem, item, &child, missing, confirmed_missing, errors);
                }
                return;
            }
            for (key, field_schema) in fields {
                let child = join_path(path, key);
                match (object.get(key), missing.contains(child.as_str())) {
                    (None, true) => {
                        confirmed_missing.insert(child);
                    }
                    (None, false) => errors.push(format!("{child}: missing field")),
                    (Some(_), true) => errors.push(format!(
                        "{child}: listed in missing_fields but now present; drop it from the list"
                    )),
                    (Some(item), false) => validate_shape(
                        field_schema,
                        item,
                        &child,
                        missing,
                        confirmed_missing,
                        errors,
                    ),
                }
            }
        }
        _ => errors.push(format!("{at}: invalid schema node {schema}")),
    }
}

/// Checks one response body against its declared contract.
fn check_response(response: &ResponseSpec, content_type: Option<&str>, body: &[u8]) -> Vec<String> {
    let mut errors = Vec::new();
    match response.content {
        Content::Text => {
            if std::str::from_utf8(body).is_err() {
                errors.push("expected a UTF-8 text body".into());
            }
        }
        Content::Bittorrent => {
            if content_type != Some("application/x-bittorrent") {
                errors.push(format!(
                    "expected content-type application/x-bittorrent, got {content_type:?}"
                ));
            }
            if body.first() != Some(&b'd') {
                errors.push("expected a bencoded dictionary body".into());
            }
        }
        Content::Json => {
            let value: Value = match serde_json::from_slice(body) {
                Ok(value) => value,
                Err(error) => return vec![format!("body is not JSON ({error})")],
            };
            let schema = response
                .schema
                .as_ref()
                .expect("json responses carry a schema");
            let missing: HashSet<&str> =
                response.missing_fields.iter().map(String::as_str).collect();
            let mut confirmed = HashSet::new();
            validate_shape(schema, &value, "", &missing, &mut confirmed, &mut errors);
            for field in &response.missing_fields {
                if !confirmed.contains(field)
                    && !errors.iter().any(|e| e.starts_with(field.as_str()))
                {
                    errors.push(format!(
                        "{field}: listed in missing_fields but never checked \
                         (path does not exist in the schema, or the probe returned no elements)"
                    ));
                }
            }
        }
    }
    errors
}

pub(super) struct ProbeResponse {
    pub(super) status: StatusCode,
    pub(super) content_type: Option<String>,
    pub(super) body: bytes::Bytes,
}

impl ProbeResponse {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "expected JSON ({e}): {:?}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap()
    }
}

pub(super) async fn send(
    router: &axum::Router,
    request: http::Request<axum::body::Body>,
) -> ProbeResponse {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(response.into_body(), 16 << 20)
        .await
        .unwrap();
    ProbeResponse {
        status,
        content_type,
        body,
    }
}

async fn get(router: &axum::Router, uri: &str) -> ProbeResponse {
    let request = http::Request::builder()
        .uri(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    send(router, request).await
}

async fn post_form(router: &axum::Router, uri: &str, form: &[(&str, &str)]) -> ProbeResponse {
    let request = http::Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(axum::body::Body::from(
            serde_urlencoded::to_string(form).unwrap(),
        ))
        .unwrap();
    send(router, request).await
}

/// Builds a two-file torrent (with one tracker) over data written under
/// `<root>/payload`, so a torrent saved to `root` finds its data complete.
async fn make_payload_torrent(root: &Path) -> Vec<u8> {
    let src = root.join("payload");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.bin"), vec![0x5a; 40_000]).unwrap();
    std::fs::write(src.join("sub").join("b.bin"), vec![0xa5; 20_000]).unwrap();
    create_torrent(
        &src,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            trackers: vec!["http://127.0.0.1:1/announce".to_owned()],
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec()
}

async fn wait_for_finished_creator_task(router: &axum::Router, task_id: &str) {
    for _ in 0..200 {
        let status = get(router, &format!("/torrentcreator/status?taskID={task_id}")).await;
        if status.json()[0]["status"] == "Finished" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("torrent creator task {task_id} did not finish");
}

/// Probes every routed endpoint against a session holding a live torrent (with
/// a category, a tag and a finished torrent-creator task, so collections are
/// non-empty) and checks each response against the declared contract.
#[tokio::test]
async fn routed_responses_match_declared_shapes() {
    let spec = parse_spec();
    let (router, session, output) = make_router().await;

    let torrent = make_payload_torrent(output.path()).await;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    let hash = handle.info_hash().as_string();

    let setup = [
        ("/torrents/createCategory", vec![("category", "movies")]),
        (
            "/torrents/setCategory",
            vec![("hashes", hash.as_str()), ("category", "movies")],
        ),
        (
            "/torrents/addTags",
            vec![("hashes", hash.as_str()), ("tags", "hd")],
        ),
    ];
    for (uri, form) in &setup {
        assert_eq!(
            post_form(&router, uri, form).await.status,
            StatusCode::OK,
            "{uri}"
        );
    }
    let source = output.path().join("payload");
    let source = source.to_string_lossy();
    let task = post_form(
        &router,
        "/torrentcreator/addTask",
        &[("sourcePath", &source)],
    )
    .await;
    let task_id = task.json()["taskID"].as_str().unwrap().to_owned();
    wait_for_finished_creator_task(&router, &task_id).await;

    let mut violations = Vec::new();
    let mut checked = 0;
    // GETs first: they are read-only and must all succeed against the fixture.
    for ep in spec.endpoints.iter().filter(|ep| ep.method == "GET") {
        let Some(contract) = &ep.response else {
            continue;
        };
        let uri = format!(
            "/{}?hash={hash}&hashes={hash}&taskID={task_id}",
            ep.endpoint
        );
        let response = get(&router, &uri).await;
        if response.status != StatusCode::OK {
            violations.push(format!("GET {}: status {}", ep.endpoint, response.status));
            continue;
        }
        checked += 1;
        for error in check_response(contract, response.content_type.as_deref(), &response.body) {
            violations.push(format!("GET {}: {error}", ep.endpoint));
        }
    }
    // POSTs with a hash that matches nothing, so the probes cannot disturb
    // each other; only successful responses are shape-checked (many POSTs
    // legitimately reject a probe that lacks their required fields).
    let unknown = "0".repeat(40);
    for ep in spec.endpoints.iter().filter(|ep| ep.method == "POST") {
        let Some(contract) = &ep.response else {
            continue;
        };
        let form = [
            ("hashes", unknown.as_str()),
            ("hash", unknown.as_str()),
            ("sourcePath", &source),
        ];
        let response = post_form(&router, &format!("/{}", ep.endpoint), &form).await;
        if !response.status.is_success() {
            continue;
        }
        checked += 1;
        for error in check_response(contract, response.content_type.as_deref(), &response.body) {
            violations.push(format!("POST {}: {error}", ep.endpoint));
        }
    }

    assert!(
        violations.is_empty(),
        "qbit compat responses drifted from qbit_parity_spec.json ({checked} endpoints checked):\n{}",
        violations.join("\n")
    );
}

#[test]
fn shape_validator_reports_drift() {
    let schema: Value = serde_json::json!([{"a": "integer", "b": {"*": "string"}, "c": "boolean"}]);
    let check = |value: Value, missing: &[&str]| {
        let response = ResponseSpec {
            content: Content::Json,
            schema: Some(schema.clone()),
            missing_fields: missing.iter().map(|s| s.to_string()).collect(),
        };
        check_response(&response, None, value.to_string().as_bytes())
    };

    assert!(
        check(
            serde_json::json!([{"a": 1, "b": {"x": "y"}, "c": true, "extra": 0}]),
            &[]
        )
        .is_empty()
    );
    assert!(check(serde_json::json!([{"a": 1, "b": {}}]), &["*.c"]).is_empty());

    let errors = check(serde_json::json!([{"a": 1.5, "b": {"x": 1}}]), &[]);
    assert_eq!(
        errors,
        vec![
            "*.a: expected integer, got 1.5",
            "*.b.*: expected string, got 1",
            "*.c: missing field",
        ]
    );
    // A declared gap that has been filled must be dropped from the list.
    assert_eq!(
        check(serde_json::json!([{"a": 1, "b": {}, "c": false}]), &["*.c"]).len(),
        1
    );
    // A declared gap that can never be checked is reported too.
    assert_eq!(
        check(
            serde_json::json!([{"a": 1, "b": {}, "c": false}]),
            &["*.nope"]
        )
        .len(),
        1
    );
    assert!(check_schema_syntax(&serde_json::json!({"*": "string", "x": "integer"})).is_err());
    assert!(check_schema_syntax(&serde_json::json!("float")).is_err());
}

// ---------------------------------------------------------------------------
// Behavioural lifecycle over the compat router
// ---------------------------------------------------------------------------

pub(super) fn multipart_add_body(
    boundary: &str,
    torrent: &[u8],
    fields: &[(&str, &str)],
) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\ncontent-disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\ncontent-disposition: form-data; name=\"torrents\"; filename=\"t.torrent\"\r\n\
             content-type: application/x-bittorrent\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(torrent);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Polls `torrents/info` for `hash` until `done` accepts the torrent's entry,
/// validating every response against the declared `torrents/info` shape.
async fn wait_for_info(
    router: &axum::Router,
    info_contract: &ResponseSpec,
    hash: &str,
    what: &str,
    done: impl Fn(&Value) -> bool,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..400 {
        let response = get(router, &format!("/torrents/info?hashes={hash}")).await;
        assert_eq!(response.status, StatusCode::OK);
        let errors = check_response(
            info_contract,
            response.content_type.as_deref(),
            &response.body,
        );
        assert!(errors.is_empty(), "torrents/info shape drift: {errors:?}");
        last = response.json()[0].clone();
        if done(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for torrent to be {what}; last torrents/info entry: {last}");
}

fn state_of(torrent: &Value) -> &str {
    torrent["state"].as_str().unwrap_or_default()
}

/// Number of torrents `torrents/info?filter=<filter>` returns.
async fn filter_count(router: &axum::Router, filter: &str) -> usize {
    let list = get(router, &format!("/torrents/info?filter={filter}"))
        .await
        .json();
    list.as_array()
        .map(Vec::len)
        .unwrap_or_else(|| panic!("{list}"))
}

/// add -> stop -> start -> recheck -> setCategory -> delete, driven entirely
/// through the compat router against a real session, asserting what a client
/// would observe through `torrents/info` after each step.
#[tokio::test]
async fn torrent_lifecycle_through_compat_router() {
    let spec = parse_spec();
    let info_contract = spec
        .endpoints
        .iter()
        .find(|ep| ep.endpoint == "torrents/info")
        .and_then(|ep| ep.response.as_ref())
        .expect("torrents/info declares a response");
    let (router, _session, output) = make_router().await;
    let torrent = make_payload_torrent(output.path()).await;
    let save_path = output.path().to_string_lossy().into_owned();

    // add (stopped, into the directory that already holds the payload)
    let boundary = "qbitLifecycleBoundary";
    let request = http::Request::builder()
        .method("POST")
        .uri("/torrents/add")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(multipart_add_body(
            boundary,
            &torrent,
            &[("savepath", &save_path), ("stopped", "true")],
        )))
        .unwrap();
    let added = send(&router, request).await;
    assert_eq!((added.status, added.text()), (StatusCode::OK, "Ok."));

    let list = get(&router, "/torrents/info").await.json();
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    let hash = list[0]["hash"].as_str().unwrap().to_owned();
    let added = wait_for_info(&router, info_contract, &hash, "stopped after add", |t| {
        state_of(t).starts_with("stopped")
    })
    .await;
    assert_eq!(added["name"], "payload");
    assert_eq!(added["total_size"], 60_000);
    assert_eq!(added["category"], "");
    assert_eq!(get(&router, "/torrents/count").await.json(), 1);

    // stop is idempotent on an already-stopped torrent
    let form = [("hashes", hash.as_str())];
    assert_eq!(
        post_form(&router, "/torrents/stop", &form).await.text(),
        "Ok."
    );
    wait_for_info(&router, info_contract, &hash, "still stopped", |t| {
        state_of(t).starts_with("stopped")
    })
    .await;

    // start
    assert_eq!(
        post_form(&router, "/torrents/start", &form).await.text(),
        "Ok."
    );
    wait_for_info(&router, info_contract, &hash, "running", |t| {
        matches!(state_of(t), "downloading" | "uploading")
    })
    .await;
    // 2.11 filter vocabulary: a running torrent is `running`, not `stopped`
    assert_eq!(filter_count(&router, "running").await, 1);
    assert_eq!(filter_count(&router, "resumed").await, 1);
    assert_eq!(filter_count(&router, "stopped").await, 0);
    assert_eq!(filter_count(&router, "paused").await, 0);

    // stop
    assert_eq!(
        post_form(&router, "/torrents/stop", &form).await.text(),
        "Ok."
    );
    wait_for_info(&router, info_contract, &hash, "stopped", |t| {
        state_of(t).starts_with("stopped")
    })
    .await;

    // recheck: the payload is complete on disk, so the torrent ends up stopped
    // and seeding-complete.
    assert_eq!(
        post_form(&router, "/torrents/recheck", &form).await.text(),
        "Ok."
    );
    let rechecked = wait_for_info(&router, info_contract, &hash, "rechecked complete", |t| {
        state_of(t) == "stoppedUP" && t["progress"] == 1.0
    })
    .await;
    assert_eq!(rechecked["amount_left"], 0);
    assert_eq!(filter_count(&router, "stopped").await, 1);
    assert_eq!(filter_count(&router, "paused").await, 1);
    assert_eq!(filter_count(&router, "running").await, 0);
    assert_eq!(filter_count(&router, "resumed").await, 0);

    // setCategory, visible both on the torrent and through the category filter
    let created = post_form(
        &router,
        "/torrents/createCategory",
        &[("category", "movies")],
    )
    .await;
    assert_eq!(created.status, StatusCode::OK);
    let set = post_form(
        &router,
        "/torrents/setCategory",
        &[("hashes", hash.as_str()), ("category", "movies")],
    )
    .await;
    assert_eq!(set.text(), "Ok.");
    wait_for_info(&router, info_contract, &hash, "categorised", |t| {
        t["category"] == "movies"
    })
    .await;
    let filtered = get(&router, "/torrents/info?category=movies").await.json();
    assert_eq!(filtered.as_array().map(Vec::len), Some(1), "{filtered}");
    let categories = get(&router, "/torrents/categories").await.json();
    assert_eq!(categories["movies"]["name"], "movies", "{categories}");

    // delete without files: the torrent is gone, the payload stays on disk
    let deleted = post_form(
        &router,
        "/torrents/delete",
        &[("hashes", hash.as_str()), ("deleteFiles", "false")],
    )
    .await;
    assert_eq!(deleted.text(), "Ok.");
    assert_eq!(
        get(&router, "/torrents/info").await.json(),
        serde_json::json!([])
    );
    assert_eq!(get(&router, "/torrents/count").await.json(), 0);
    assert!(output.path().join("payload").join("a.bin").exists());
}

/// `torrents/setShareLimits` reaches the session: the limits show up in
/// `torrents/info` and the session's own view, and the policy pass stops a
/// running seed that has reached its ratio.
#[tokio::test]
async fn share_limits_through_compat_router_are_enforced() {
    let spec = parse_spec();
    let info_contract = spec
        .endpoints
        .iter()
        .find(|ep| ep.endpoint == "torrents/info")
        .and_then(|ep| ep.response.as_ref())
        .expect("torrents/info declares a response");
    let (router, session, output) = make_router().await;
    let torrent = make_payload_torrent(output.path()).await;
    let save_path = output.path().to_string_lossy().into_owned();

    let boundary = "qbitShareLimitsBoundary";
    let request = http::Request::builder()
        .method("POST")
        .uri("/torrents/add")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(multipart_add_body(
            boundary,
            &torrent,
            &[("savepath", &save_path), ("stopped", "true")],
        )))
        .unwrap();
    assert_eq!(send(&router, request).await.text(), "Ok.");
    let list = get(&router, "/torrents/info").await.json();
    let hash = list[0]["hash"].as_str().unwrap().to_owned();
    assert_eq!(list[0]["ratio_limit"], -1.0);
    assert_eq!(list[0]["inactive_seeding_time_limit"], -1);

    let set = post_form(
        &router,
        "/torrents/setShareLimits",
        &[
            ("hashes", hash.as_str()),
            ("ratioLimit", "2.5"),
            ("seedingTimeLimit", "120"),
            ("inactiveSeedingTimeLimit", "30"),
        ],
    )
    .await;
    assert_eq!(set.text(), "Ok.");
    let info = wait_for_info(&router, info_contract, &hash, "limited", |t| {
        t["ratio_limit"] == 2.5
    })
    .await;
    assert_eq!(info["max_ratio"], 2.5);
    assert_eq!(info["seeding_time_limit"], 120);
    assert_eq!(info["max_seeding_time"], 120);
    assert_eq!(info["inactive_seeding_time_limit"], 30);
    assert_eq!(info["max_inactive_seeding_time"], 30);
    let handle = session.with_torrents(|it| it.next().map(|(_, h)| h.clone()).unwrap());
    let limits = session.share_limits(&handle);
    assert_eq!(limits.max_ratio, Some(2.5));
    assert_eq!(limits.max_seeding_time, Some(Duration::from_secs(7200)));
    assert_eq!(
        limits.max_inactive_seeding_time,
        Some(Duration::from_secs(1800))
    );

    // An older client omits inactiveSeedingTimeLimit: that limit is kept, and
    // -1 / -2 clear the others.
    let form = [
        ("hashes", hash.as_str()),
        ("ratioLimit", "-2"),
        ("seedingTimeLimit", "-1"),
    ];
    assert_eq!(
        post_form(&router, "/torrents/setShareLimits", &form)
            .await
            .text(),
        "Ok."
    );
    let limits = session.share_limits(&handle);
    assert_eq!(limits.max_ratio, None);
    assert_eq!(limits.max_seeding_time, None);
    assert_eq!(
        limits.max_inactive_seeding_time,
        Some(Duration::from_secs(1800))
    );

    // Recheck the complete payload, start seeding, then a zero ratio trips.
    let form = [("hashes", hash.as_str())];
    post_form(&router, "/torrents/recheck", &form).await;
    wait_for_info(&router, info_contract, &hash, "rechecked complete", |t| {
        state_of(t) == "stoppedUP"
    })
    .await;
    post_form(&router, "/torrents/start", &form).await;
    wait_for_info(&router, info_contract, &hash, "seeding", |t| {
        state_of(t) == "uploading" || state_of(t) == "stalledUP"
    })
    .await;
    let set = post_form(
        &router,
        "/torrents/setShareLimits",
        &[
            ("hashes", hash.as_str()),
            ("ratioLimit", "0"),
            ("seedingTimeLimit", "-1"),
            ("inactiveSeedingTimeLimit", "-1"),
        ],
    )
    .await;
    assert_eq!(set.text(), "Ok.");
    session.policy_pass().await;
    let stopped = wait_for_info(&router, info_contract, &hash, "stopped by ratio", |t| {
        state_of(t) == "stoppedUP"
    })
    .await;
    assert_eq!(stopped["ratio_limit"], 0.0);
}

/// Session limits and alternative-speed mode, driven through the compat
/// router and cross-checked against the native session state the
/// `/torrents/limits` and `/speed/alt` APIs read, so both views agree.
#[tokio::test]
async fn transfer_limits_and_alt_speed_through_compat_router() {
    let (router, session, _output) = make_router().await;
    let text = |r: ProbeResponse| r.text().to_owned();

    // normal limits: set via compat, visible via compat and the native limiter
    assert_eq!(
        text(
            post_form(
                &router,
                "/transfer/setDownloadLimit",
                &[("limit", "100000")]
            )
            .await
        ),
        "Ok."
    );
    assert_eq!(
        text(post_form(&router, "/transfer/setUploadLimit", &[("limit", "50000")]).await),
        "Ok."
    );
    assert_eq!(
        get(&router, "/transfer/downloadLimit").await.json(),
        100_000
    );
    assert_eq!(get(&router, "/transfer/uploadLimit").await.json(), 50_000);
    let config = session.ratelimits.get_config();
    assert_eq!(config.download_bps.map(|v| v.get()), Some(100_000));
    assert_eq!(config.upload_bps.map(|v| v.get()), Some(50_000));
    let info = get(&router, "/transfer/info").await.json();
    assert_eq!(
        (info["dl_rate_limit"].clone(), info["up_rate_limit"].clone()),
        (100_000.into(), 50_000.into()),
        "{info}"
    );

    // setSpeedLimits requires all four fields
    let partial = post_form(&router, "/transfer/setSpeedLimits", &[("dl_limit", "1")]).await;
    assert_eq!(partial.status, StatusCode::BAD_REQUEST);
    let set = post_form(
        &router,
        "/transfer/setSpeedLimits",
        &[
            ("dl_limit", "200000"),
            ("up_limit", "0"),
            ("alt_dl_limit", "10000"),
            ("alt_up_limit", "5000"),
        ],
    )
    .await;
    assert_eq!((set.status, set.text()), (StatusCode::OK, "Ok."));
    let limits = serde_json::json!({
        "dl_limit": 200_000, "up_limit": 0, "alt_dl_limit": 10_000, "alt_up_limit": 5_000
    });
    assert_eq!(
        get(&router, "/transfer/getSpeedLimits").await.json(),
        limits
    );
    let alt = session.alt_speed_status().config;
    assert_eq!(
        (alt.download_rate, alt.upload_rate),
        (Some(10_000), Some(5_000))
    );

    // alternative mode: the live limiter switches to the alt rates, while
    // getSpeedLimits keeps reporting the configured normal limits
    assert_eq!(get(&router, "/transfer/speedLimitsMode").await.text(), "0");
    assert_eq!(
        text(post_form(&router, "/transfer/toggleSpeedLimitsMode", &[]).await),
        "Ok."
    );
    assert!(session.alt_speed_enabled());
    assert_eq!(get(&router, "/transfer/speedLimitsMode").await.text(), "1");
    assert_eq!(get(&router, "/transfer/downloadLimit").await.json(), 10_000);
    assert_eq!(
        get(&router, "/transfer/getSpeedLimits").await.json(),
        limits
    );
    let maindata = get(&router, "/sync/maindata").await.json();
    assert_eq!(maindata["server_state"]["use_alt_speed_limits"], true);
    assert_eq!(maindata["server_state"]["dl_rate_limit"], 10_000);

    // a normal-limit change while alt mode is on applies once it is turned off
    post_form(&router, "/transfer/setUploadLimit", &[("limit", "7000")]).await;
    assert_eq!(get(&router, "/transfer/uploadLimit").await.json(), 5_000);
    assert_eq!(
        text(post_form(&router, "/transfer/setSpeedLimitsMode", &[("mode", "0")]).await),
        "Ok."
    );
    assert!(!session.alt_speed_enabled());
    assert_eq!(get(&router, "/transfer/speedLimitsMode").await.text(), "0");
    assert_eq!(
        get(&router, "/transfer/downloadLimit").await.json(),
        200_000
    );
    assert_eq!(get(&router, "/transfer/uploadLimit").await.json(), 7_000);

    // DHT is disabled in the fixture, so dht_nodes reports 0 rather than failing
    assert_eq!(get(&router, "/transfer/info").await.json()["dht_nodes"], 0);
}

fn response_contract<'a>(spec: &'a Spec, endpoint: &str) -> &'a ResponseSpec {
    spec.endpoints
        .iter()
        .find(|ep| ep.endpoint == endpoint)
        .and_then(|ep| ep.response.as_ref())
        .unwrap_or_else(|| panic!("{endpoint} declares a response"))
}

/// `sync/maindata` rid deltas: a full snapshot at rid 0, then only what
/// changed (tags, the touched torrent field, removals) against a held rid,
/// and a full snapshot again for an unknown rid.
#[tokio::test]
async fn sync_maindata_rid_deltas_through_compat_router() {
    let spec = parse_spec();
    let contract = response_contract(&spec, "sync/maindata");
    let (router, session, output) = make_router().await;
    let torrent = make_payload_torrent(output.path()).await;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    handle.wait_until_initialized().await.unwrap();
    let hash = handle.info_hash().as_string();

    let full = get(&router, "/sync/maindata?rid=0").await;
    let errors = check_response(contract, full.content_type.as_deref(), &full.body);
    assert!(errors.is_empty(), "{errors:?}");
    let full = full.json();
    assert_eq!(full["full_update"], true);
    assert_eq!(full["torrents"][&hash]["name"], "payload");
    let rid = full["rid"].as_u64().unwrap();

    post_form(
        &router,
        "/torrents/addTags",
        &[("hashes", &hash), ("tags", "hd")],
    )
    .await;
    let delta = get(&router, &format!("/sync/maindata?rid={rid}"))
        .await
        .json();
    assert_eq!(delta.get("full_update"), None, "{delta}");
    assert_eq!(delta["tags"], serde_json::json!(["hd"]), "{delta}");
    let changed = &delta["torrents"][&hash];
    assert_eq!(changed["tags"], "hd", "{delta}");
    assert_eq!(
        changed.get("name"),
        None,
        "unchanged fields are omitted: {delta}"
    );
    assert_eq!(delta.get("categories"), None, "{delta}");
    let rid = delta["rid"].as_u64().unwrap();

    post_form(&router, "/torrents/deleteTags", &[("tags", "hd")]).await;
    post_form(
        &router,
        "/torrents/delete",
        &[("hashes", &hash), ("deleteFiles", "false")],
    )
    .await;
    let delta = get(&router, &format!("/sync/maindata?rid={rid}"))
        .await
        .json();
    assert_eq!(delta["tags_removed"], serde_json::json!(["hd"]), "{delta}");
    assert_eq!(
        delta["torrents_removed"],
        serde_json::json!([hash]),
        "{delta}"
    );
    let rid = delta["rid"].as_u64().unwrap();

    // a stale rid from before the deletes still diffs against its own snapshot
    let stale = get(&router, &format!("/sync/maindata?rid={}", rid - 1))
        .await
        .json();
    assert_eq!(
        stale["torrents_removed"],
        serde_json::json!([hash]),
        "{stale}"
    );
    // an rid the server never issued yields a full update
    let unknown = get(&router, "/sync/maindata?rid=999999").await.json();
    assert_eq!(unknown["full_update"], true);
    assert_eq!(unknown["torrents"], serde_json::json!({}));
}

/// `sync/torrentPeers` against a real peer: a second session downloads the
/// fixture torrent from the router's session, which must list it (shape
/// checked against the spec), report its traffic in deltas, and list it in
/// `peers_removed` once it goes away.
#[tokio::test]
async fn sync_torrent_peers_through_compat_router() {
    let spec = parse_spec();
    let contract = response_contract(&spec, "sync/torrentPeers");
    let (router, seeder, output) = make_router().await;
    let torrent = make_payload_torrent(output.path()).await;
    let handle = seeder
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions {
                overwrite: true,
                // files land directly in output_folder, so point it at the payload
                output_folder: Some(output.path().join("payload").to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), handle.wait_until_completed())
        .await
        .expect("seeder verifies its payload")
        .unwrap();
    let hash = handle.info_hash().as_string();
    let uri = |rid: u64| format!("/sync/torrentPeers?hash={hash}&rid={rid}");

    let empty = get(&router, &uri(0)).await.json();
    assert_eq!(empty["full_update"], true);
    assert_eq!(empty["peers"], serde_json::json!({}));
    assert_eq!(
        get(
            &router,
            &format!("/sync/torrentPeers?hash={}", "0".repeat(40))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    let leecher_dir = tempfile::TempDir::with_prefix("qbit_parity_leecher").unwrap();
    let leecher = Session::new_with_opts(
        leecher_dir.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seeder_addr = seeder.listen_addr().expect("seeder listens");
    let leech = leecher
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                initial_peers: Some(vec![seeder_addr]),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), leech.wait_until_completed())
        .await
        .expect("leecher completes")
        .unwrap();

    // the leecher shows up as a peer with its upload counted
    let mut full = None;
    for _ in 0..200 {
        let response = get(&router, &uri(0)).await;
        let body = response.json();
        let uploaded = body["peers"]
            .as_object()
            .and_then(|peers| peers.values().next())
            .and_then(|peer| peer["uploaded"].as_u64())
            .unwrap_or(0);
        if uploaded > 0 {
            let errors = check_response(contract, response.content_type.as_deref(), &response.body);
            assert!(errors.is_empty(), "{errors:?}");
            full = Some(body);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let full = full.expect("leecher listed among the seeder's peers");
    let (addr, peer) = full["peers"].as_object().unwrap().iter().next().unwrap();
    assert_eq!(peer["ip"], "127.0.0.1");
    assert_eq!(addr, &format!("127.0.0.1:{}", peer["port"]));
    assert_eq!(peer["connection"], "BT");
    let rid = full["rid"].as_u64().unwrap();

    // an rid from another scope (maindata) does not diff against peers
    let main_rid = get(&router, "/sync/maindata").await.json()["rid"]
        .as_u64()
        .unwrap();
    assert_eq!(
        get(&router, &uri(main_rid)).await.json()["full_update"],
        true
    );

    // once the leecher goes away the seeder reports the peer removed
    tokio::time::timeout(
        Duration::from_secs(10),
        leecher.delete(leech.id().into(), false),
    )
    .await
    .expect("leecher deletes its torrent")
    .unwrap();
    drop(leecher);
    let mut removed = false;
    for _ in 0..400 {
        let delta = get(&router, &uri(rid)).await.json();
        assert_eq!(delta.get("full_update"), None, "{delta}");
        if delta["peers_removed"] == serde_json::json!([addr]) {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(removed, "peer {addr} never reported in peers_removed");
}

/// Per-torrent speed limits: set at add time and through `setDownloadLimit` /
/// `setUploadLimit`, read back through `downloadLimit` / `uploadLimit`,
/// `torrents/info` and `torrents/properties`, and cross-checked against the
/// torrent's native limits.
#[tokio::test]
async fn torrent_limits_through_compat_router() {
    let (router, session, output) = make_router().await;
    let torrent = make_payload_torrent(output.path()).await;
    let save_path = output.path().to_string_lossy().into_owned();

    let boundary = "qbitLimitsBoundary";
    let request = http::Request::builder()
        .method("POST")
        .uri("/torrents/add")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(multipart_add_body(
            boundary,
            &torrent,
            &[
                ("savepath", &save_path),
                ("stopped", "true"),
                ("dlLimit", "300000"),
                ("upLimit", "-1"),
            ],
        )))
        .unwrap();
    assert_eq!(send(&router, request).await.status, StatusCode::OK);
    let hash = get(&router, "/torrents/info").await.json()[0]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let info = || async {
        get(&router, &format!("/torrents/info?hashes={hash}"))
            .await
            .json()[0]
            .clone()
    };
    assert_eq!(info().await["dl_limit"], 300_000);
    assert_eq!(info().await["up_limit"], -1);

    let ok = |r: ProbeResponse| assert_eq!(r.text(), "Ok.");
    ok(post_form(
        &router,
        "/torrents/setUploadLimit",
        &[("hashes", &hash), ("limit", "50000")],
    )
    .await);
    ok(post_form(
        &router,
        "/torrents/setDownloadLimit",
        &[("hashes", &hash), ("limit", "0")],
    )
    .await);

    // Setting one direction keeps the other.
    assert_eq!(info().await["dl_limit"], -1);
    assert_eq!(info().await["up_limit"], 50_000);
    let props = get(&router, &format!("/torrents/properties?hash={hash}"))
        .await
        .json();
    assert_eq!(
        (props["dl_limit"].clone(), props["up_limit"].clone()),
        (Value::from(-1), Value::from(50_000))
    );
    assert_eq!(
        get(&router, &format!("/torrents/uploadLimit?hashes={hash}"))
            .await
            .json()[&hash],
        50_000
    );
    assert_eq!(
        get(&router, &format!("/torrents/downloadLimit?hashes={hash}"))
            .await
            .json()[&hash],
        0
    );

    let handle = session
        .get(crate::api::TorrentIdOrHash::parse(&hash).unwrap())
        .unwrap();
    assert_eq!(
        handle.rate_limits().upload_bps.map(|b| b.get()),
        Some(50_000)
    );
    assert_eq!(handle.rate_limits().download_bps, None);
}

/// The bridge endpoints that read or adjust one torrent through existing
/// backend calls (`count`, `defaultSavePath`, `properties` metadata, `export`,
/// `pieceHashes`, `pieceStates`, `filePrio`, `addPeers`, `rename`) plus the
/// `clientdata` key/value store.
#[tokio::test]
async fn bridge_endpoints_through_compat_router() {
    let spec = parse_spec();
    let info_contract = response_contract(&spec, "torrents/info");
    let (router, _session, output) = make_router().await;
    // Give the outer dictionary the descriptive fields `properties` reports.
    // The info dictionary (and so the info hash) is unchanged.
    let created = make_payload_torrent(output.path()).await;
    assert_eq!(created[0], b'd');
    let mut torrent =
        b"d7:comment9:a comment10:created by9:swarmtest13:creation datei1700000000e".to_vec();
    torrent.extend_from_slice(&created[1..]);
    let save_path = output.path().to_string_lossy().into_owned();

    let boundary = "qbitBridgeBoundary";
    let request = http::Request::builder()
        .method("POST")
        .uri("/torrents/add")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(multipart_add_body(
            boundary,
            &torrent,
            &[("savepath", &save_path), ("stopped", "true")],
        )))
        .unwrap();
    assert_eq!(send(&router, request).await.status, StatusCode::OK);
    assert_eq!(get(&router, "/torrents/count").await.json(), 1);
    assert!(!get(&router, "/app/defaultSavePath").await.text().is_empty());

    let hash = get(&router, "/torrents/info").await.json()[0]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    // The payload is already on disk, so the initial check finds it complete.
    wait_for_info(&router, info_contract, &hash, "fully checked", |t| {
        t["progress"].as_f64() == Some(1.0)
    })
    .await;

    let props = get(&router, &format!("/torrents/properties?hash={hash}"))
        .await
        .json();
    assert_eq!(props["comment"], "a comment");
    assert_eq!(props["created_by"], "swarmtest");
    assert_eq!(props["creation_date"], 1_700_000_000);
    let pieces = usize::try_from(props["pieces_num"].as_u64().unwrap()).unwrap();
    assert_eq!(pieces, 4, "60 000 bytes in 16 KiB pieces");

    let exported = get(&router, &format!("/torrents/export?hash={hash}")).await;
    assert_eq!(exported.status, StatusCode::OK);
    assert_eq!(
        exported.content_type.as_deref(),
        Some("application/x-bittorrent")
    );
    assert_eq!(exported.body, torrent);

    let hashes = get(&router, &format!("/torrents/pieceHashes?hash={hash}"))
        .await
        .json();
    let hashes = hashes.as_array().unwrap();
    assert_eq!(hashes.len(), pieces);
    assert!(hashes.iter().all(|h| {
        let h = h.as_str().unwrap();
        h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit())
    }));
    let states = get(&router, &format!("/torrents/pieceStates?hash={hash}"))
        .await
        .json();
    assert_eq!(states, Value::from(vec![2; pieces]));

    // filePrio: 0 skips a file, any other level includes it again.
    let file_priorities = || async {
        get(&router, &format!("/torrents/files?hash={hash}"))
            .await
            .json()
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["priority"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(file_priorities().await, vec![1, 1]);
    let set_prio = |id: &'static str, priority: &'static str| {
        let router = &router;
        let hash = &hash;
        async move {
            post_form(
                router,
                "/torrents/filePrio",
                &[("hash", hash), ("id", id), ("priority", priority)],
            )
            .await
            .status
        }
    };
    assert_eq!(set_prio("1", "0").await, StatusCode::OK);
    assert_eq!(file_priorities().await, vec![1, 0]);
    assert_eq!(set_prio("1", "7").await, StatusCode::OK);
    assert_eq!(file_priorities().await, vec![1, 1]);
    assert_eq!(set_prio("5", "0").await, StatusCode::CONFLICT);

    let added = post_form(
        &router,
        "/torrents/addPeers",
        &[("hashes", &hash), ("peers", "127.0.0.1:6881|[::1]:6881")],
    )
    .await;
    assert_eq!((added.status, added.text()), (StatusCode::OK, "Ok."));

    let renamed = post_form(
        &router,
        "/torrents/rename",
        &[("hash", &hash), ("name", "renamed")],
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(
        get(&router, &format!("/torrents/info?hashes={hash}"))
            .await
            .json()[0]["name"],
        "renamed"
    );

    // clientdata: set, read back (all or by key), and remove with null.
    assert_eq!(
        get(&router, "/clientdata/load").await.json(),
        serde_json::json!({})
    );
    let store = |data: &'static str| {
        let router = &router;
        async move { post_form(router, "/clientdata/store", &[("data", data)]).await }
    };
    assert_eq!(
        store(r#"{"theme":"dark","layout":{"cols":[1,2]}}"#)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        get(&router, "/clientdata/load").await.json(),
        serde_json::json!({"theme": "dark", "layout": {"cols": [1, 2]}})
    );
    assert_eq!(
        get(
            &router,
            "/clientdata/load?keys=%5B%22theme%22%2C%22nope%22%5D"
        )
        .await
        .json(),
        serde_json::json!({"theme": "dark"})
    );
    assert_eq!(store(r#"{"theme":null}"#).await.status, StatusCode::OK);
    assert_eq!(
        get(&router, "/clientdata/load").await.json(),
        serde_json::json!({"layout": {"cols": [1, 2]}})
    );
    assert_eq!(store("[1]").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        post_form(&router, "/clientdata/store", &[]).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&router, "/clientdata/load?keys=notjson").await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn ban_peers_and_logs_through_compat_router() {
    let (router, session, _output) = make_router().await;

    let banned = post_form(
        &router,
        "/transfer/banPeers",
        &[("peers", "10.9.8.7:6881|[2001:db8::1]:51413|bogus")],
    )
    .await;
    assert_eq!(banned.status, StatusCode::OK);
    assert!(session.is_peer_banned("10.9.8.7".parse().unwrap()));
    assert!(session.is_peer_banned("2001:db8::1".parse().unwrap()));
    // IPv4-mapped IPv6 (dual-stack listeners) matches the IPv4 ban.
    assert!(session.is_peer_banned("::ffff:10.9.8.7".parse().unwrap()));
    assert!(!session.is_peer_banned("10.9.8.6".parse().unwrap()));
    assert_eq!(
        post_form(&router, "/transfer/banPeers", &[("peers", "")])
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    let peers = get(&router, "/log/peers").await.json();
    let entry = peers
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["ip"] == "10.9.8.7")
        .expect("ban recorded in log/peers");
    assert_eq!(entry["blocked"], true);
    let id = entry["id"].as_i64().unwrap();
    let newer = get(&router, &format!("/log/peers?last_known_id={id}"))
        .await
        .json();
    assert!(
        newer
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["id"].as_i64().unwrap() > id)
    );

    // The ring is process-wide; find our own entries by message.
    let ring = crate::log_ring::global();
    ring.push(
        crate::log_ring::LogSeverity::Warning,
        "wi22-probe-warning".into(),
    );
    ring.push(
        crate::log_ring::LogSeverity::Normal,
        "wi22-probe-normal".into(),
    );
    let messages = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|e| e["message"].as_str().unwrap().to_owned())
            .filter(|m| m.starts_with("wi22-probe"))
            .collect()
    };
    let all = get(&router, "/log/main").await.json();
    assert_eq!(
        messages(&all),
        vec!["wi22-probe-warning", "wi22-probe-normal"]
    );
    let warnings_only = get(&router, "/log/main?normal=false&info=false&critical=false")
        .await
        .json();
    assert_eq!(messages(&warnings_only), vec!["wi22-probe-warning"]);
    let warn_id = all
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["message"] == "wi22-probe-warning")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let after = get(&router, &format!("/log/main?last_known_id={warn_id}"))
        .await
        .json();
    assert_eq!(messages(&after), vec!["wi22-probe-normal"]);
}
