//! qBittorrent WebUI API parity checker.
//!
//! `qbit_parity_spec.json` is a machine-readable inventory of the upstream
//! qBittorrent WebUI API v2 surface (extracted from the qBittorrent sources),
//! annotated with the parity status of each endpoint in our compat layer
//! (`qbit_compat.rs`). These tests keep the spec and the actual router from
//! drifting apart:
//!
//! - every endpoint marked `full` or `partial` must be routed;
//! - every endpoint marked `missing` or `out_of_scope` must return 404;
//! - every route literal in `qbit_compat.rs` must be tracked in the spec;
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
const COMPAT_SOURCE: &str = include_str!("qbit_compat.rs");

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

async fn make_router() -> (axum::Router, Arc<Session>, tempfile::TempDir) {
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
    let api_state = Arc::new(HttpApi::new(
        api,
        Some(HttpApiOptions {
            allow_create: true,
            ..Default::default()
        }),
    ));
    (
        super::qbit_compat::make_qbit_router(api_state),
        session,
        output,
    )
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

/// Every `.route("...")` literal in qbit_compat.rs must correspond to a spec
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
        "routes in qbit_compat.rs with no full/partial entry in qbit_parity_spec.json: {untracked:?}"
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

struct ProbeResponse {
    status: StatusCode,
    content_type: Option<String>,
    body: bytes::Bytes,
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

async fn send(router: &axum::Router, request: http::Request<axum::body::Body>) -> ProbeResponse {
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

fn multipart_add_body(boundary: &str, torrent: &[u8], fields: &[(&str, &str)]) -> Vec<u8> {
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
