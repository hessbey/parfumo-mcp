//! Settings of the tools, read from environment variables: where the database is, how
//! fast Parfumo may be asked, what may be written. The settings of the HTTP server (address,
//! public URL, tokens, canary) are in `parfumo-mcp-server`.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use crate::crypto::MasterKey;

pub const ENV_MASTER_KEY: &str = "PARFUMO_MCP_MASTER_KEY";
pub const ENV_MASTER_KEY_FILE: &str = "PARFUMO_MCP_MASTER_KEY_FILE";
pub const ENV_PUBLIC_WRITE_CAPS: &str = "PARFUMO_MCP_PUBLIC_WRITE_CAPS";
pub const ENV_WRITE_RECIPIENTS: &str = "PARFUMO_MCP_WRITE_RECIPIENTS";

/// Tools that send writes other Parfumo members can see, with their initial daily cap per
/// user (cahier des charges, section 9: 20 private messages, 10 forum posts, 10 reviews or
/// statements; the others are proposals of jalon 4).
pub const PUBLIC_WRITE_TOOLS: &[(&str, u32)] = &[
    ("parfumo_send_message", 20),
    ("parfumo_forum_post", 10),
    ("parfumo_write_review", 10),
    ("parfumo_write_statement", 10),
    ("parfumo_wall", 10),
    ("parfumo_blog", 5),
    ("parfumo_souk_manage_offer", 10),
    ("parfumo_manage_follow", 30),
    ("parfumo_update_profile", 20),
    ("parfumo_log_wear", 20),
    ("parfumo_rate_perfume", 100),
    ("parfumo_classify_perfume", 100),
];

/// Daily caps of the publicly visible writes, per tool and per user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteCaps(BTreeMap<&'static str, u32>);

impl Default for WriteCaps {
    fn default() -> Self {
        Self(PUBLIC_WRITE_TOOLS.iter().copied().collect())
    }
}

impl WriteCaps {
    /// `tool=cap` pairs separated by commas override the initial values; `0` forbids the
    /// publicly visible writes of a tool. Unknown tools and bad numbers are refused.
    pub fn parse(spec: Option<&str>) -> Result<Self, String> {
        let mut caps = Self::default();
        for pair in spec.unwrap_or_default().split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (tool, cap) = pair.split_once('=').ok_or_else(|| format!("{ENV_PUBLIC_WRITE_CAPS}: `{pair}` is not `tool=cap`"))?;
            let cap: u32 = cap.trim().parse().map_err(|_| format!("{ENV_PUBLIC_WRITE_CAPS}: the cap of `{}` must be a whole number", tool.trim()))?;
            let slot = caps.0.iter_mut().find(|(name, _)| **name == tool.trim()).ok_or_else(|| {
                format!(
                    "{ENV_PUBLIC_WRITE_CAPS}: `{}` is not a tool with publicly visible writes ({})",
                    tool.trim(),
                    PUBLIC_WRITE_TOOLS.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")
                )
            })?;
            *slot.1 = cap;
        }
        Ok(caps)
    }

    /// Daily cap of a tool; a tool without publicly visible writes has none.
    pub fn of(&self, tool: &str) -> Option<u32> {
        self.0.get(tool).copied()
    }

    pub fn all(&self) -> impl Iterator<Item = (&'static str, u32)> + '_ {
        PUBLIC_WRITE_TOOLS.iter().filter_map(|(tool, _)| self.of(tool).map(|cap| (*tool, cap)))
    }
}

#[derive(Clone)]
pub struct Config {
    /// SQLite file. Default: the volume of the Docker image; the local binary uses the
    /// user's data directory instead.
    pub db_path: PathBuf,
    pub parfumo_base_url: String,
    /// Minimum interval between two requests of one user to Parfumo.
    pub user_min_interval: Duration,
    /// Upper bound of the random delay added to each request.
    pub user_jitter: Duration,
    /// Global budget, all users together, in requests per second.
    pub global_rps: u32,
    /// Whether tools may really send writes that other Parfumo members can see (type P).
    /// Off by default: such tools then only answer as a dry run.
    pub public_writes: bool,
    /// Daily caps of those writes, per tool and per user.
    pub public_write_caps: WriteCaps,
    /// When not empty, the only Parfumo members (lower-case nicknames) a tool may write to:
    /// private message, wall post, follow. Empty: no restriction.
    pub write_recipients: Vec<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("db_path", &self.db_path).field("parfumo_base_url", &self.parfumo_base_url).finish_non_exhaustive()
    }
}

/// A variable of the environment, trimmed; empty counts as unset.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn env_u64(name: &str, default: u64) -> Result<u64, String> {
    match env(name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{name} must be a positive integer")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            db_path: env("PARFUMO_MCP_DB").unwrap_or_else(|| "/data/parfumo-mcp.db".into()).into(),
            parfumo_base_url: env("PARFUMO_BASE_URL").unwrap_or_else(|| "https://www.parfumo.com".into()).trim_end_matches('/').to_string(),
            user_min_interval: Duration::from_millis(env_u64("PARFUMO_MCP_USER_MIN_INTERVAL_MS", 1000)?.max(1000)),
            user_jitter: Duration::from_millis(env_u64("PARFUMO_MCP_USER_JITTER_MS", 400)?),
            global_rps: env_u64("PARFUMO_MCP_GLOBAL_RPS", 2)?.clamp(1, 2) as u32,
            public_writes: env("PARFUMO_MCP_PUBLIC_WRITES").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")),
            public_write_caps: WriteCaps::parse(env(ENV_PUBLIC_WRITE_CAPS).as_deref())?,
            write_recipients: parse_recipients(env(ENV_WRITE_RECIPIENTS).as_deref()),
        })
    }

    /// Whether a tool may write to this Parfumo member (see `write_recipients`).
    pub fn recipient_allowed(&self, nick: &str) -> bool {
        self.write_recipients.is_empty() || self.write_recipients.iter().any(|r| r.eq_ignore_ascii_case(nick.trim()))
    }
}

pub fn parse_recipients(spec: Option<&str>) -> Vec<String> {
    spec.unwrap_or_default().split(',').map(|n| n.trim().trim_start_matches('@').to_lowercase()).filter(|n| !n.is_empty()).collect()
}

/// Master key from `PARFUMO_MCP_MASTER_KEY` (base64, 32 bytes) or from the file named by
/// `PARFUMO_MCP_MASTER_KEY_FILE` (Docker secret). It is never stored in the database.
pub fn master_key_from_env() -> Result<MasterKey, String> {
    let raw = match (env(ENV_MASTER_KEY), env(ENV_MASTER_KEY_FILE)) {
        (Some(v), _) => v,
        (None, Some(path)) => std::fs::read_to_string(&path).map_err(|e| format!("cannot read {ENV_MASTER_KEY_FILE}: {e}"))?.trim().to_string(),
        (None, None) => {
            return Err(format!("{ENV_MASTER_KEY} (or {ENV_MASTER_KEY_FILE}) is required; generate one with `parfumo-mcp-server keygen`"));
        }
    };
    MasterKey::from_base64(&raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_default_to_the_initial_values_and_can_be_overridden() {
        let caps = WriteCaps::parse(None).unwrap();
        assert_eq!(
            (caps.of("parfumo_send_message"), caps.of("parfumo_forum_post"), caps.of("parfumo_write_review"), caps.of("parfumo_write_statement")),
            (Some(20), Some(10), Some(10), Some(10))
        );
        assert_eq!(caps.of("parfumo_get_perfume"), None, "a read has no cap");
        let caps = WriteCaps::parse(Some(" parfumo_send_message = 3, parfumo_wall=0 ,")).unwrap();
        assert_eq!((caps.of("parfumo_send_message"), caps.of("parfumo_wall"), caps.of("parfumo_forum_post")), (Some(3), Some(0), Some(10)));
        assert_eq!(caps.all().count(), PUBLIC_WRITE_TOOLS.len());
        for bad in ["parfumo_send_message", "parfumo_send_message=-1", "parfumo_send_message=many", "parfumo_get_perfume=5"] {
            let err = WriteCaps::parse(Some(bad)).unwrap_err();
            assert!(err.contains(ENV_PUBLIC_WRITE_CAPS), "{bad}: {err}");
        }
    }

    #[test]
    fn recipients_are_normalised() {
        assert_eq!(parse_recipients(Some(" Alice, @bob ,,")), ["alice", "bob"]);
        assert!(parse_recipients(None).is_empty());
    }
}
