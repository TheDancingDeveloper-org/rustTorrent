//! Optional TOML config file, given with `--config` or `RTBIT_CONFIG`.
//!
//! Only non-secret settings belong here. Passwords and API keys stay in the credentials store
//! or the environment. Precedence, highest first: the command line, the environment, this file,
//! then the built-in default. `rtbit config dump` prints the settings that result.

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::path::Path;

use anyhow::Context;
use clap::{ValueEnum, parser::ValueSource};
use serde::{Deserialize, Serialize};

use crate::{LogLevel, Opts};

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_api: Option<HttpApiSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexarr: Option<IndexarrSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingSection>,
}

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpApiSection {
    /// Same as `--http-api-listen-addr`, e.g. "127.0.0.1:3030".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_addr: Option<SocketAddr>,
    /// Same as `--http-api-allow-create`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_create: Option<bool>,
}

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announce_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_ip: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_dht: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_tcp_listen: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_tcp_connect: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv4_only: Option<bool>,
}

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsSection {
    /// Bytes per second. Same as `--ratelimit-download`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bps: Option<NonZeroU32>,
    /// Bytes per second. Same as `--ratelimit-upload`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_bps: Option<NonZeroU32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_limit: Option<usize>,
}

/// The API key is deliberately absent: it is a secret and belongs in the environment.
#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexarrSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSection {
    /// trace, debug, info, warn or error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Reads and parses the file. Unknown keys are errors, so a typo cannot pass silently.
pub fn load(path: &Path) -> anyhow::Result<ConfigFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config file {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid config file {}", path.display()))
}

/// Fills in each setting the file sets, unless the command line or the environment already set
/// it. `matches` must be the matches `opts` was parsed from.
pub fn apply(opts: &mut Opts, matches: &clap::ArgMatches, file: &ConfigFile) -> anyhow::Result<()> {
    // Set by the user for this run, so the file must not override it.
    let from_file = |id: &str| {
        !matches!(
            matches.value_source(id),
            Some(ValueSource::CommandLine | ValueSource::EnvVariable)
        )
    };

    if let Some(section) = &file.http_api {
        if from_file("http_api_listen_addr")
            && let Some(v) = section.listen_addr
        {
            opts.http_api_listen_addr = Some(v);
        }
        if from_file("http_api_allow_create")
            && let Some(v) = section.allow_create
        {
            opts.http_api_allow_create = v;
        }
    }

    if let Some(section) = &file.network {
        if from_file("listen_port")
            && let Some(v) = section.listen_port
        {
            opts.listen_port = Some(v);
        }
        if from_file("announce_port")
            && let Some(v) = section.announce_port
        {
            opts.announce_port = Some(v);
        }
        if from_file("listen_ip")
            && let Some(v) = section.listen_ip
        {
            opts.listen_ip = v;
        }
        if from_file("disable_dht")
            && let Some(v) = section.disable_dht
        {
            opts.disable_dht = v;
        }
        if from_file("disable_tcp_listen")
            && let Some(v) = section.disable_tcp_listen
        {
            opts.disable_tcp_listen = v;
        }
        if from_file("disable_tcp_connect")
            && let Some(v) = section.disable_tcp_connect
        {
            opts.disable_tcp_connect = v;
        }
        if from_file("ipv4_only")
            && let Some(v) = section.ipv4_only
        {
            opts.ipv4_only = v;
        }
    }

    if let Some(section) = &file.limits {
        if from_file("ratelimit_download_bps")
            && let Some(v) = section.download_bps
        {
            opts.ratelimit_download_bps = Some(v);
        }
        if from_file("ratelimit_upload_bps")
            && let Some(v) = section.upload_bps
        {
            opts.ratelimit_upload_bps = Some(v);
        }
        if from_file("peer_limit")
            && let Some(v) = section.peer_limit
        {
            opts.peer_limit = Some(v);
        }
    }

    if let Some(section) = &file.indexarr {
        if from_file("indexarr_enabled")
            && let Some(v) = section.enabled
        {
            opts.indexarr_enabled = v;
        }
        if from_file("indexarr_url")
            && let Some(v) = &section.url
        {
            opts.indexarr_url = Some(v.clone());
        }
    }

    if let Some(section) = &file.logging {
        if from_file("log_level")
            && let Some(v) = &section.level
        {
            let level = LogLevel::from_str(v, true)
                .map_err(|_| anyhow::anyhow!("invalid log level {v:?} in config file"))?;
            opts.log_level = Some(level);
        }
        if from_file("log_file")
            && let Some(v) = &section.file
        {
            opts.log_file = Some(v.clone());
        }
    }

    Ok(())
}

/// The settings as they now stand, for `rtbit config dump`. Secrets are never included, so the
/// output is safe to paste into a bug report.
pub fn dump(opts: &Opts) -> ConfigFile {
    ConfigFile {
        http_api: Some(HttpApiSection {
            listen_addr: opts.http_api_listen_addr,
            allow_create: Some(opts.http_api_allow_create),
        }),
        network: Some(NetworkSection {
            listen_port: opts.listen_port,
            announce_port: opts.announce_port,
            listen_ip: Some(opts.listen_ip),
            disable_dht: Some(opts.disable_dht),
            disable_tcp_listen: Some(opts.disable_tcp_listen),
            disable_tcp_connect: Some(opts.disable_tcp_connect),
            ipv4_only: Some(opts.ipv4_only),
        }),
        limits: Some(LimitsSection {
            download_bps: opts.ratelimit_download_bps,
            upload_bps: opts.ratelimit_upload_bps,
            peer_limit: opts.peer_limit,
        }),
        indexarr: Some(IndexarrSection {
            enabled: Some(opts.indexarr_enabled),
            url: opts.indexarr_url.clone(),
        }),
        logging: Some(LoggingSection {
            level: opts
                .log_level
                .and_then(|level| level.to_possible_value())
                .map(|value| value.get_name().to_owned()),
            file: opts.log_file.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches};

    use super::*;

    fn parse_cli(args: &[&str]) -> (Opts, clap::ArgMatches) {
        let matches = Opts::command()
            .try_get_matches_from(args)
            .expect("arguments parse");
        let opts = Opts::from_arg_matches(&matches).expect("options build");
        (opts, matches)
    }

    #[test]
    fn sample_file_parses_and_round_trips() {
        let text = r#"
            [http_api]
            listen_addr = "127.0.0.1:3030"
            allow_create = true

            [network]
            listen_port = 6881
            disable_dht = true
            listen_ip = "0.0.0.0"

            [limits]
            download_bps = 1000000
            peer_limit = 50

            [logging]
            level = "debug"
        "#;
        let file: ConfigFile = toml::from_str(text).expect("parse");
        assert_eq!(
            file.network.as_ref().and_then(|n| n.listen_port),
            Some(6881)
        );
        let dumped = toml::to_string_pretty(&file).expect("serialize");
        let again: ConfigFile = toml::from_str(&dumped).expect("reparse");
        assert_eq!(again, file);
    }

    #[test]
    fn unknown_keys_and_secrets_are_rejected() {
        assert!(toml::from_str::<ConfigFile>("bogus = true\n").is_err());
        assert!(toml::from_str::<ConfigFile>("[network]\nlisten_prot = 1\n").is_err());
        assert!(
            toml::from_str::<ConfigFile>("[indexarr]\napi_key = \"secret\"\n").is_err(),
            "secrets must not be accepted from the file"
        );
    }

    #[test]
    fn command_line_wins_over_the_file_and_the_file_fills_the_rest() {
        let file: ConfigFile = toml::from_str(
            "[network]\nlisten_port = 7000\ndisable_dht = true\n[limits]\npeer_limit = 9\n",
        )
        .expect("parse");
        let (mut opts, matches) =
            parse_cli(&["rtbit", "--listen-port", "6000", "completions", "bash"]);
        apply(&mut opts, &matches, &file).expect("apply");
        assert_eq!(opts.listen_port, Some(6000), "the command line wins");
        assert!(
            opts.disable_dht,
            "the file sets what the command line does not"
        );
        assert_eq!(opts.peer_limit, Some(9));
    }

    #[test]
    fn invalid_log_level_is_an_error() {
        let file: ConfigFile = toml::from_str("[logging]\nlevel = \"loud\"\n").expect("parse");
        let (mut opts, matches) = parse_cli(&["rtbit", "completions", "bash"]);
        assert!(apply(&mut opts, &matches, &file).is_err());
    }

    #[test]
    fn dump_reflects_the_settings_and_leaves_out_secrets() {
        let (opts, _) = parse_cli(&[
            "rtbit",
            "--listen-port",
            "6000",
            "--indexarr-api-key",
            "hunter2",
            "completions",
            "bash",
        ]);
        let text = toml::to_string_pretty(&dump(&opts)).expect("serialize");
        assert!(text.contains("listen_port = 6000"));
        assert!(!text.contains("hunter2"), "the API key must not be dumped");
        let reparsed: ConfigFile = toml::from_str(&text).expect("dump is valid config");
        assert_eq!(reparsed, dump(&opts));
    }
}
