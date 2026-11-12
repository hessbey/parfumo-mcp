//! Settings of the HTTP server, read from environment variables: where it listens, its
//! public address, how long tokens live, the daily canary. The settings of the tools are
//! in `parfumo_mcp_core::config`.

use std::{net::SocketAddr, time::Duration};

use parfumo_mcp_core::config::{env, env_u64};

pub const ENV_CANARY_HOUR: &str = "PARFUMO_MCP_CANARY_HOUR";

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Listening address inside the container.
    pub bind: SocketAddr,
    /// Public HTTPS origin of this server, without trailing slash (e.g. `https://mcp.example.com`).
    pub public_url: String,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    /// Hour (UTC) of the daily canary run, `None` when it is switched off.
    pub canary_hour: Option<u8>,
    /// Server user whose linked Parfumo account (the test account) the canary also reads.
    pub canary_user: Option<String>,
    /// Where to post an alert when the canary result is not `ok`.
    pub canary_webhook: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let bind = env("PARFUMO_MCP_BIND").unwrap_or_else(|| "0.0.0.0:8080".into());
        let bind: SocketAddr = bind.parse().map_err(|_| "PARFUMO_MCP_BIND must be host:port".to_string())?;
        let public_url = env("PARFUMO_MCP_PUBLIC_URL").unwrap_or_else(|| format!("http://localhost:{}", bind.port()));
        let public_url = public_url.trim_end_matches('/').to_string();
        let parsed = url::Url::parse(&public_url).map_err(|_| "PARFUMO_MCP_PUBLIC_URL is not a valid URL".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err("PARFUMO_MCP_PUBLIC_URL must be an http(s) URL".into());
        }
        Ok(Self {
            bind,
            public_url,
            // Tokens stay short-lived whatever the setting: one minute to one day for an
            // access token, one hour to ninety days for a refresh token.
            access_token_ttl: Duration::from_secs(env_u64("PARFUMO_MCP_ACCESS_TOKEN_TTL_SECS", 3600)?.clamp(60, 86_400)),
            refresh_token_ttl: Duration::from_secs(env_u64("PARFUMO_MCP_REFRESH_TOKEN_TTL_SECS", 30 * 86400)?.clamp(3600, 90 * 86_400)),
            canary_hour: parse_canary_hour(env(ENV_CANARY_HOUR).as_deref())?,
            canary_user: env("PARFUMO_MCP_CANARY_USER"),
            canary_webhook: match env("PARFUMO_MCP_CANARY_WEBHOOK") {
                Some(u) if u.starts_with("https://") || u.starts_with("http://") => Some(u),
                Some(_) => return Err("PARFUMO_MCP_CANARY_WEBHOOK must be an http(s) URL".into()),
                None => None,
            },
        })
    }

    /// `host[:port]` of the public URL, used for Host validation.
    pub fn public_authority(&self) -> String {
        let u = url::Url::parse(&self.public_url).expect("validated in from_env");
        match u.port() {
            Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()),
            None => u.host_str().unwrap_or_default().to_string(),
        }
    }

    /// Canonical URI of the protected MCP resource.
    pub fn resource_url(&self) -> String {
        format!("{}/mcp", self.public_url)
    }

    /// Page where a user links a Parfumo account.
    pub fn link_page(&self) -> String {
        format!("{}/link", self.public_url)
    }
}

/// `PARFUMO_MCP_CANARY_HOUR`: an hour of the day in UTC (default 4), or `off`.
pub fn parse_canary_hour(spec: Option<&str>) -> Result<Option<u8>, String> {
    match spec.map(str::to_ascii_lowercase).as_deref() {
        None => Ok(Some(4)),
        Some("off" | "none" | "false" | "no") => Ok(None),
        Some(v) => v
            .parse::<u8>()
            .ok()
            .filter(|h| *h < 24)
            .map(Some)
            .ok_or_else(|| format!("{ENV_CANARY_HOUR} must be an hour from 0 to 23 (UTC), or `off`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_hour_defaults_to_four_and_can_be_switched_off() {
        assert_eq!(parse_canary_hour(None), Ok(Some(4)));
        assert_eq!(parse_canary_hour(Some("23")), Ok(Some(23)));
        assert_eq!(parse_canary_hour(Some("OFF")), Ok(None));
        for bad in ["24", "-1", "noon"] {
            assert!(parse_canary_hour(Some(bad)).unwrap_err().contains(ENV_CANARY_HOUR), "{bad}");
        }
    }
}
