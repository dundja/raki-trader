//! Runtime configuration: `config/traderd.toml` plus a few env overrides.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use raki_core::{Asset, Source};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Mirrors `config/traderd.toml`. It holds no secrets: PolyBolt credentials come only from env.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Raw files go under `<data_dir>/raw`. Relative paths resolve against the working directory.
    pub data_dir: PathBuf,
    /// Overridden by `RAKI_DATABASE_URL`.
    pub database_url: String,
    /// Overridden by `RAKI_API_BIND`.
    pub api_bind: SocketAddr,
    pub cors_origin: String,
    pub assets: Vec<Asset>,
    pub feeds: FeedsConfig,
    pub health: HealthConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedsConfig {
    pub clob: bool,
    pub rtds_legacy: bool,
    pub polybolt: Polybolt,
    pub binance: bool,
    pub coinbase: bool,
}

/// `polybolt = true | false | "auto"`. Anything but `false` still needs the three `POLY_API_*`
/// env vars: without credentials PolyBolt stays off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polybolt {
    Auto,
    On,
    Off,
}

impl<'de> Deserialize<'de> for Polybolt {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Bool(true) => Ok(Polybolt::On),
            Raw::Bool(false) => Ok(Polybolt::Off),
            Raw::Str(s) if s == "auto" => Ok(Polybolt::Auto),
            Raw::Str(s) => Err(serde::de::Error::custom(format!(
                "polybolt must be true, false or \"auto\", got {s:?}"
            ))),
        }
    }
}

impl Serialize for Polybolt {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Polybolt::Auto => s.serialize_str("auto"),
            Polybolt::On => s.serialize_bool(true),
            Polybolt::Off => s.serialize_bool(false),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthConfig {
    pub clock_max_offset_ms: f64,
    pub ntp_servers: Vec<String>,
    pub ntp_interval_s: u64,
    pub geoblock_interval_s: u64,
    /// A feed silent for longer than this is stale. Missing sources default to 10 s.
    pub stale_ms: HashMap<Source, i64>,
}

/// Reads the config at `path` and applies `RAKI_DATABASE_URL` / `RAKI_API_BIND` when set.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    load_with_env(path, |k| std::env::var(k).ok())
}

/// [`load`] with an injectable env lookup, so tests never mutate the process environment.
pub fn load_with_env(path: &Path, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    let mut cfg: Config =
        toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))?;
    if let Some(url) = env("RAKI_DATABASE_URL").filter(|v| !v.is_empty()) {
        cfg.database_url = url;
    }
    if let Some(bind) = env("RAKI_API_BIND").filter(|v| !v.is_empty()) {
        cfg.api_bind = bind
            .parse()
            .with_context(|| format!("RAKI_API_BIND {bind:?} is not an address"))?;
    }
    anyhow::ensure!(!cfg.assets.is_empty(), "config lists no assets");
    Ok(cfg)
}

impl Config {
    /// The config as JSON for the raw `SessionStart` record, with any database password masked
    /// (the env override may carry a real one).
    pub fn redacted_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).expect("config serializes");
        v["database_url"] = redact_url_password(&self.database_url).into();
        v
    }
}

/// `scheme://user:secret@host/..` becomes `scheme://user:***@host/..`. Other strings pass through.
pub fn redact_url_password(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else {
        return url.to_owned();
    };
    let rest = &url[scheme_end..];
    let Some(at) = rest.rfind('@') else {
        return url.to_owned();
    };
    match rest[..at].find(':') {
        Some(colon) => format!(
            "{}{}:***{}",
            &url[..scheme_end],
            &rest[..colon],
            &rest[at..]
        ),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_config() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/traderd.toml")
    }

    #[test]
    fn config_loads_repo_config() {
        let cfg = load(&repo_config()).unwrap();
        assert_eq!(cfg.assets, [Asset::Btc, Asset::Eth, Asset::Sol]);
        assert_eq!(cfg.health.clock_max_offset_ms, 50.0);
        assert_eq!(cfg.feeds.polybolt, Polybolt::Auto);
        assert_eq!(cfg.health.stale_ms.get(&Source::RtdsLegacy), Some(&3_000));
        assert_eq!(cfg.api_bind.to_string(), "127.0.0.1:8787");
    }

    #[test]
    fn env_override_database_url() {
        let env = |k: &str| match k {
            "RAKI_DATABASE_URL" => Some("postgres://u:p@db:5432/x".to_owned()),
            "RAKI_API_BIND" => Some("127.0.0.1:9999".to_owned()),
            _ => None,
        };
        let cfg = load_with_env(&repo_config(), env).unwrap();
        assert_eq!(cfg.database_url, "postgres://u:p@db:5432/x");
        assert_eq!(cfg.api_bind.to_string(), "127.0.0.1:9999");
    }

    #[test]
    fn polybolt_setting_parses_bool_or_auto() {
        let parse = |v: &str| toml::from_str::<HashMap<String, Polybolt>>(&format!("p = {v}"));
        assert_eq!(parse("true").unwrap()["p"], Polybolt::On);
        assert_eq!(parse("false").unwrap()["p"], Polybolt::Off);
        assert_eq!(parse("\"auto\"").unwrap()["p"], Polybolt::Auto);
        assert!(parse("\"yes\"").is_err());
    }

    #[test]
    fn redacts_database_password() {
        assert_eq!(
            redact_url_password("postgres://raki:raki@127.0.0.1:5433/raki"),
            "postgres://raki:***@127.0.0.1:5433/raki"
        );
        assert_eq!(
            redact_url_password("postgres://raki@host/db"),
            "postgres://raki@host/db"
        );
        assert_eq!(redact_url_password("not a url"), "not a url");
        let cfg = load(&repo_config()).unwrap();
        let v = cfg.redacted_json();
        assert_eq!(v["database_url"], "postgres://raki:***@127.0.0.1:5433/raki");
        assert_eq!(v["feeds"]["polybolt"], "auto");
    }
}
