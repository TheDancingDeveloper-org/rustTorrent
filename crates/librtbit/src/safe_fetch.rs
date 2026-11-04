//! The one fetcher for server-side requests to URLs that came from outside
//! (torrent URL adds, RSS feeds, blocklists).
//!
//! DNS is resolved once per hop, every resolved address must be public unless
//! the host (or literal IP) is on the allowlist, and the connection is pinned
//! to the vetted addresses so a second lookup cannot swap in a private one.
//! Redirects are followed manually (at most [`MAX_REDIRECTS`]), each hop
//! re-checked. Body size and total time are capped.

use std::net::{IpAddr, SocketAddr};
use std::sync::RwLock;
use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::{Client, Response, Url, header};

pub const MAX_REDIRECTS: usize = 5;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Hosts and literal IPs that may be fetched even when they resolve to a
/// non-public address. Empty by default.
static ALLOWLIST: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Replace the allowlist of hosts/IPs exempt from the non-public address check.
pub fn set_allowlist(entries: impl IntoIterator<Item = String>) {
    let list = entries
        .into_iter()
        .map(|e| e.to_ascii_lowercase())
        .collect();
    *ALLOWLIST.write().unwrap_or_else(|e| e.into_inner()) = list;
}

fn is_allowlisted(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    ALLOWLIST
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&host)
}

#[derive(Debug, Clone)]
pub struct FetchLimits {
    pub timeout: Duration,
    pub max_bytes: usize,
    pub max_redirects: usize,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_bytes: DEFAULT_MAX_BYTES,
            max_redirects: MAX_REDIRECTS,
        }
    }
}

/// Refuse add-torrent sources that a remote caller (HTTP API, RSS item,
/// feed-supplied link) must not choose. Only magnets, bare info hashes and
/// `http`/`https` URLs pass; `file://` is a local-file read and stays limited
/// to operator-side entry points (CLI arguments).
pub fn check_remote_add_url(url: &str) -> anyhow::Result<()> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let bare_hash = url.len() == 40 && url.bytes().all(|b| b.is_ascii_hexdigit());
    if bare_hash
        || lower.starts_with("magnet:")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
    {
        return Ok(());
    }
    bail!("only magnet, http and https URLs can be added remotely")
}

/// True for loopback, private, link-local, documentation, broadcast and other
/// addresses a feed must never be fetched from.
pub fn is_forbidden_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // "This network" 0.0.0.0/8 (is_unspecified covers only 0.0.0.0).
                || a == 0
                // Carrier-grade NAT, 100.64.0.0/10.
                || (a == 100 && (64..128).contains(&b))
                // Benchmarking 198.18.0.0/15.
                || (a == 198 && (b == 18 || b == 19))
                // Reserved for future use, 240.0.0.0/4.
                || a >= 240
        }
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local fc00::/7 and link local fe80::/10.
                || (seg0 & 0xfe00) == 0xfc00
                || (seg0 & 0xffc0) == 0xfe80
                // NAT64 well-known prefixes (including the RFC 6052 /96
                // form and the RFC 6052 /48 form used by local translators).
                || is_nat64(v6)
                || (seg0 == 0x64 && v6.segments()[1] == 0xff9b && v6.segments()[2] == 1)
                // Teredo tunnels can expose a client behind a private NAT.
                || seg0 == 0x2001 && v6.segments()[1] == 0
                // IPv4-compatible ::a.b.c.d and 6to4 2002::/16.
                || is_ipv4_compatible(v6)
                || seg0 == 0x2002
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_forbidden_address(IpAddr::V4(mapped)))
        }
    }
}

fn is_nat64(v6: std::net::Ipv6Addr) -> bool {
    let s = v6.segments();
    s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0
}

/// Deprecated IPv4-compatible form `::a.b.c.d` (not `::` and not `::1`).
fn is_ipv4_compatible(v6: std::net::Ipv6Addr) -> bool {
    let s = v6.segments();
    s[0] == 0
        && s[1] == 0
        && s[2] == 0
        && s[3] == 0
        && s[4] == 0
        && s[5] == 0
        && !(s[6] == 0 && (s[7] == 0 || s[7] == 1))
}

/// Resolve the host once and return the addresses it may be connected to.
async fn vet_host(url: &Url) -> anyhow::Result<(String, u16, Vec<IpAddr>)> {
    if !matches!(url.scheme(), "http" | "https") {
        bail!("URL must use http or https");
    }
    let host = url.host_str().context("URL has no host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let bare = host.trim_matches(['[', ']']);
    let addrs: Vec<IpAddr> = if let Ok(ip) = bare.parse::<IpAddr>() {
        vec![ip]
    } else {
        tokio::net::lookup_host((bare, port))
            .await
            .with_context(|| format!("could not resolve {host}"))?
            .map(|a| a.ip())
            .collect()
    };
    if addrs.is_empty() {
        bail!("{host} resolved to no addresses");
    }
    if !is_allowlisted(&host) && addrs.iter().any(|ip| is_forbidden_address(*ip)) {
        bail!("{host} points at a private, loopback or link-local address");
    }
    Ok((host, port, addrs))
}

/// Send a GET following at most `limits.max_redirects` checked redirects.
pub async fn get(url: &str, limits: &FetchLimits) -> anyhow::Result<Response> {
    let mut current = Url::parse(url).context("invalid URL")?;
    for _ in 0..=limits.max_redirects {
        let (host, port, addrs) = vet_host(&current).await?;
        let mut builder = Client::builder()
            .timeout(limits.timeout)
            // Resolve-pinning only protects direct connections. Never let an
            // environment proxy route around the vetted destination.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        for ip in addrs {
            builder = builder.resolve(&host, SocketAddr::new(ip, port));
        }
        let response = builder.build()?.get(current.clone()).send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("redirect with no Location header")?;
            current = current
                .join(location)
                .context("redirect Location is not a URL")?;
            continue;
        }
        return Ok(response);
    }
    bail!("redirected more than {} times", limits.max_redirects)
}

/// Stream a response body, refusing it once it passes `max_bytes`.
pub async fn read_capped(mut response: Response, max_bytes: usize) -> anyhow::Result<Vec<u8>> {
    if let Some(len) = response.content_length()
        && len > max_bytes as u64
    {
        bail!("Content-Length {len} exceeds the {max_bytes} byte cap");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            bail!("body exceeds the {max_bytes} byte cap");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// GET `url` and return its body, enforcing every limit. Non-2xx is an error.
pub async fn fetch_bytes(url: &str, limits: &FetchLimits) -> anyhow::Result<Vec<u8>> {
    let response = get(url, limits).await?;
    if !response.status().is_success() {
        bail!("GET {url} returned {}", response.status());
    }
    read_capped(response, limits.max_bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // The allowlist is process-wide, so tests that touch it run one at a time.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn serve(response: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf).await;
                    let _ = s.write_all(response.as_bytes()).await;
                });
            }
        });
        port
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(to: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    async fn refused(url: &str) -> String {
        let _g = SERIAL.lock().await;
        set_allowlist([]);
        format!(
            "{:#}",
            fetch_bytes(url, &FetchLimits::default()).await.unwrap_err()
        )
    }

    #[tokio::test]
    async fn refuses_loopback() {
        assert!(refused("http://127.0.0.1:1/").await.contains("private"));
        assert!(refused("http://localhost:1/").await.contains("private"));
        assert!(refused("http://[::1]:1/").await.contains("private"));
    }

    #[tokio::test]
    async fn refuses_metadata_and_link_local() {
        assert!(
            refused("http://169.254.169.254/latest/meta-data")
                .await
                .contains("private")
        );
        assert!(refused("http://[fe80::1]/").await.contains("private"));
    }

    #[tokio::test]
    async fn refuses_private_ranges() {
        for u in [
            "http://10.0.0.1/",
            "http://172.16.5.5/",
            "http://192.168.1.1/",
            "http://100.64.0.1/",
        ] {
            assert!(refused(u).await.contains("private"), "{u}");
        }
    }

    #[tokio::test]
    async fn refuses_unspecified_multicast_and_reserved() {
        for u in [
            "http://0.0.0.0/",
            "http://0.1.2.3/",
            "http://224.0.0.1/",
            "http://255.255.255.255/",
            "http://198.18.0.1/",
            "http://240.0.0.1/",
        ] {
            assert!(refused(u).await.contains("private"), "{u}");
        }
    }

    #[tokio::test]
    async fn refuses_ipv6_equivalents() {
        for u in [
            "http://[::]/",
            "http://[fc00::1]/",
            "http://[fd12:3456::1]/",
            "http://[ff02::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[::ffff:10.0.0.1]/",
            "http://[64:ff9b::a00:1]/",
            "http://[2002:a00:1::]/",
        ] {
            assert!(refused(u).await.contains("private"), "{u}");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(!is_forbidden_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn remote_add_url_rejects_local_schemes() {
        for u in [
            "file:///etc/passwd",
            "FILE:///x",
            "ftp://example.com/a",
            "/etc/passwd",
            "",
        ] {
            assert!(check_remote_add_url(u).is_err(), "{u}");
        }
        for u in [
            "http://example.com/a.torrent",
            "HTTPS://example.com/a.torrent",
            "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862",
            "cab507494d02ebb1178b38f2e9d7be299c86b862",
        ] {
            assert!(check_remote_add_url(u).is_ok(), "{u}");
        }
    }

    #[tokio::test]
    async fn redirect_to_loopback_hostname_is_refused() {
        let _g = SERIAL.lock().await;
        // Only the literal IP is allowlisted, so the hop to `localhost` is vetted and refused.
        let port = serve(redirect("http://localhost:1/")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let err = fetch_bytes(
            &format!("http://127.0.0.1:{port}/"),
            &FetchLimits::default(),
        )
        .await
        .unwrap_err();
        set_allowlist([]);
        assert!(format!("{err:#}").contains("private"), "{err:#}");
    }

    #[tokio::test]
    async fn redirect_to_non_http_scheme_is_refused() {
        let _g = SERIAL.lock().await;
        let port = serve(redirect("file:///etc/passwd")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let err = fetch_bytes(
            &format!("http://127.0.0.1:{port}/"),
            &FetchLimits::default(),
        )
        .await
        .unwrap_err();
        set_allowlist([]);
        assert!(format!("{err:#}").contains("http or https"), "{err:#}");
    }

    #[tokio::test]
    async fn refuses_non_http_scheme() {
        assert!(
            refused("file:///etc/passwd")
                .await
                .contains("http or https")
        );
    }

    #[tokio::test]
    async fn allowlisted_host_is_fetched() {
        let _g = SERIAL.lock().await;
        let port = serve(ok("hello")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let body = fetch_bytes(
            &format!("http://127.0.0.1:{port}/"),
            &FetchLimits::default(),
        )
        .await
        .unwrap();
        set_allowlist([]);
        assert_eq!(body, b"hello");
    }

    #[tokio::test]
    async fn redirect_to_metadata_is_refused() {
        let _g = SERIAL.lock().await;
        let port = serve(redirect("http://169.254.169.254/latest")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let err = fetch_bytes(
            &format!("http://127.0.0.1:{port}/"),
            &FetchLimits::default(),
        )
        .await
        .unwrap_err();
        set_allowlist([]);
        assert!(format!("{err:#}").contains("private"), "{err:#}");
    }

    #[tokio::test]
    async fn redirect_chain_is_capped() {
        let _g = SERIAL.lock().await;
        let port = serve(redirect("/again")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let err = fetch_bytes(
            &format!("http://127.0.0.1:{port}/"),
            &FetchLimits::default(),
        )
        .await
        .unwrap_err();
        set_allowlist([]);
        assert!(
            format!("{err:#}").contains("redirected more than 5"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn oversized_body_is_refused() {
        let _g = SERIAL.lock().await;
        let port = serve(ok("0123456789")).await;
        set_allowlist(["127.0.0.1".to_string()]);
        let limits = FetchLimits {
            max_bytes: 4,
            ..Default::default()
        };
        let err = fetch_bytes(&format!("http://127.0.0.1:{port}/"), &limits)
            .await
            .unwrap_err();
        set_allowlist([]);
        assert!(format!("{err:#}").contains("byte cap"), "{err:#}");
    }
}
