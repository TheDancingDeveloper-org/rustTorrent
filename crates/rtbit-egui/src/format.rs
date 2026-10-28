//! Sizes, speeds and state labels for the torrent grid.

use rtbit_client::{Speed, TorrentState};

/// Bytes as a short binary size. `0 B` for nothing.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A daemon speed (`mbps`, mebibytes per second) as `12.3 KiB/s`.
#[must_use]
pub fn speed(speed: &Speed) -> String {
    let bytes_per_sec = speed.mbps * 1024.0 * 1024.0;
    if bytes_per_sec < 1.0 {
        return "0 B/s".to_owned();
    }
    format!("{}/s", bytes(bytes_per_sec as u64))
}

/// The daemon's own state word, or an em dash when the row has no stats.
#[must_use]
pub fn state_label(state: Option<TorrentState>) -> String {
    state.map_or_else(|| "—".to_owned(), |state| state.label().to_owned())
}

/// `progress / total`, or just the size when the total is unknown.
#[must_use]
pub fn progress(done: u64, total: u64) -> String {
    if total == 0 {
        bytes(done)
    } else {
        format!("{} / {}", bytes(done), bytes(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_speeds() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(
            speed(&Speed {
                mbps: 0.0,
                human_readable: String::new()
            }),
            "0 B/s"
        );
        assert_eq!(state_label(Some(TorrentState::Live)), "live");
        assert_eq!(state_label(None), "—");
    }
}
