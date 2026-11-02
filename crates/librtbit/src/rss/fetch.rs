//! Fetching an RSS feed body from an untrusted URL.
//!
//! A feed URL is attacker-controlled, so it is refused when it names a
//! loopback, private, link-local or otherwise non-public address, and the body
//! read from an accepted URL is capped.

use std::net::IpAddr;

pub use crate::safe_fetch::is_forbidden_address;

/// Largest feed body that is kept. Anything longer is truncated, so a feed
/// cannot force an unbounded allocation.
pub const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;

/// Why a feed URL was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedUrlError {
    Unparseable,
    UnsupportedScheme,
    MissingHost,
    ForbiddenAddress,
}

impl std::fmt::Display for FeedUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedUrlError::Unparseable => write!(f, "feed URL is not a valid URL"),
            FeedUrlError::UnsupportedScheme => {
                write!(f, "feed URL must use http or https")
            }
            FeedUrlError::MissingHost => write!(f, "feed URL has no host"),
            FeedUrlError::ForbiddenAddress => write!(
                f,
                "feed URL points at a private, loopback or link-local address"
            ),
        }
    }
}

/// Check a feed URL before anything is fetched.
///
/// Only `http` and `https` are allowed. A host that is an IP address must be a
/// public one; a hostname is accepted here and resolved by
/// [`assert_public_addresses`] before the request is sent.
pub fn check_feed_url(url: &str) -> Result<(), FeedUrlError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| FeedUrlError::Unparseable)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(FeedUrlError::UnsupportedScheme),
    }
    match parsed.host() {
        Some(url::Host::Ipv4(v4)) => {
            if is_forbidden_address(IpAddr::V4(v4)) {
                return Err(FeedUrlError::ForbiddenAddress);
            }
        }
        Some(url::Host::Ipv6(v6)) => {
            if is_forbidden_address(IpAddr::V6(v6)) {
                return Err(FeedUrlError::ForbiddenAddress);
            }
        }
        Some(url::Host::Domain(_)) => {}
        None => return Err(FeedUrlError::MissingHost),
    }
    Ok(())
}

/// Refuse a resolved address that is not globally reachable.
pub fn assert_public_addresses(addresses: &[IpAddr]) -> Result<(), FeedUrlError> {
    if addresses.is_empty() || addresses.iter().any(|ip| is_forbidden_address(*ip)) {
        return Err(FeedUrlError::ForbiddenAddress);
    }
    Ok(())
}

/// Reject a feed body that exceeds [`MAX_FEED_BYTES`].
///
/// Truncating XML can turn a valid feed into malformed input and hide the
/// fact that the remote response exceeded our safety limit.
pub fn cap_feed_body(body: &[u8]) -> anyhow::Result<Vec<u8>> {
    if body.len() > MAX_FEED_BYTES {
        anyhow::bail!("RSS feed body exceeds the {MAX_FEED_BYTES} byte cap")
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_loopback_and_private_literals() {
        for url in [
            "http://127.0.0.1/feed.xml",
            "http://127.1.2.3/feed.xml",
            "http://10.1.2.3/feed.xml",
            "http://192.168.1.1/feed.xml",
            "http://169.254.1.1/feed.xml",
            "http://100.64.0.1/feed.xml",
            "http://[::1]/feed.xml",
            "http://[fd00::1]/feed.xml",
        ] {
            assert_eq!(
                check_feed_url(url),
                Err(FeedUrlError::ForbiddenAddress),
                "{url}"
            );
        }
    }

    #[test]
    fn refuses_non_http_schemes_and_accepts_a_public_host() {
        assert_eq!(
            check_feed_url("file:///etc/passwd"),
            Err(FeedUrlError::UnsupportedScheme)
        );
        assert_eq!(check_feed_url("not a url"), Err(FeedUrlError::Unparseable));
        assert!(check_feed_url("https://example.com/feed.xml").is_ok());
        assert!(check_feed_url("http://1.1.1.1/feed.xml").is_ok());
    }

    #[test]
    fn resolved_private_addresses_are_refused() {
        let private = ["10.0.0.5".parse().unwrap(), "1.1.1.1".parse().unwrap()];
        assert_eq!(
            assert_public_addresses(&private),
            Err(FeedUrlError::ForbiddenAddress)
        );
        let public = ["1.1.1.1".parse::<IpAddr>().unwrap()];
        assert!(assert_public_addresses(&public).is_ok());
        assert!(assert_public_addresses(&[]).is_err());
    }

    #[test]
    fn refuses_reserved_benchmark_and_embedded_ranges() {
        for url in [
            "http://0.1.2.3/feed.xml",
            "http://198.18.0.1/feed.xml",
            "http://198.19.255.1/feed.xml",
            "http://240.0.0.1/feed.xml",
            "http://255.255.255.255/feed.xml",
            "http://[64:ff9b::1.2.3.4]/feed.xml",
            "http://[64:ff9b:1::1]/feed.xml",
            "http://[2001::1]/feed.xml",
            "http://[::192.0.2.1]/feed.xml",
            "http://[2002:c000:201::]/feed.xml",
        ] {
            assert_eq!(
                check_feed_url(url),
                Err(FeedUrlError::ForbiddenAddress),
                "{url}"
            );
        }
        // A public address just outside the benchmarking block stays allowed.
        assert!(check_feed_url("http://198.17.0.1/feed.xml").is_ok());
        assert!(check_feed_url("http://198.20.0.1/feed.xml").is_ok());
        assert!(check_feed_url("http://[2001:db8::1]/feed.xml").is_ok());
    }
    #[test]
    fn caps_a_feed_body_at_the_limit() {
        let big = vec![b'a'; MAX_FEED_BYTES + 100];
        assert!(cap_feed_body(&big).is_err());
        assert_eq!(cap_feed_body(b"<rss/>").unwrap(), b"<rss/>");
    }
}
