//! Guard rails of the writes other Parfumo members can see (type P): the operator switch
//! `PARFUMO_MCP_PUBLIC_WRITES`, the daily cap per tool and per user, and the optional list
//! of members a tool may write to.
//!
//! A cap counts tool calls that really send something: a dry run, a call that finds
//! nothing to change and a call refused before sending cost nothing. The count is taken
//! just before the first request leaves, so a write Parfumo then refuses still counts.

use parfumo_client::WriteCall;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    db,
    errors::{ErrorCode, ToolError},
    tools::{Clients, ParfumoTools},
};

pub(crate) const GATE_NOTE: &str = "Publicly visible writes are disabled on this server: this call was a dry run, nothing was sent. The operator enables them with PARFUMO_MCP_PUBLIC_WRITES=true.";
pub(crate) const OWN_WORDS_NOTE: &str =
    "Parfumo forbids AI-generated text and restricts or deletes accounts that post it: the text must be the user's own words, passed on unchanged.";
pub(crate) const NEVER_SENT_NOTE: &str =
    "This Parfumo route was read in the site's own form and script but has never been exercised by this server: check the result on parfumo.com.";

/// Daily budget of one tool for the calling user.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Quota {
    /// Tool the cap applies to.
    pub tool: String,
    /// Publicly visible writes allowed per day for this user (0: forbidden).
    pub daily_cap: u32,
    /// Writes already counted today (UTC).
    pub used_today: u32,
    /// Writes still allowed today.
    pub remaining: u32,
}

/// Today's date (UTC) as the caps count it.
pub fn today() -> String {
    parfumo_client::parse::iso_date(db::now())
}

/// Seconds until the caps reset (00:00 UTC).
pub(crate) fn secs_to_reset() -> u64 {
    (86_400 - db::now().rem_euclid(86_400)) as u64
}

impl ParfumoTools {
    /// `(effective dry run, public writes enabled)` of a publicly visible write.
    pub(crate) fn public_gate(&self, dry_run: bool) -> (bool, bool) {
        let enabled = self.state.cfg.public_writes;
        (dry_run || !enabled, enabled)
    }

    /// Refuses a write aimed at a member outside the operator's list, when there is one.
    pub(crate) fn check_recipient(&self, nick: &str) -> Result<(), ToolError> {
        if self.state.cfg.recipient_allowed(nick) {
            return Ok(());
        }
        Err(ToolError::invalid(format!(
            "This server only writes to these Parfumo members: {} (operator setting PARFUMO_MCP_WRITE_RECIPIENTS). Nothing was sent to '{}'.",
            self.state.cfg.write_recipients.join(", "),
            nick.trim()
        )))
    }

    /// Today's budget of every tool with publicly visible writes, for one user.
    pub(crate) fn quotas(&self, user_id: i64) -> Vec<Quota> {
        let used = self.state.db.public_writes_on(user_id, &today()).unwrap_or_default();
        self.state
            .cfg
            .public_write_caps
            .all()
            .map(|(tool, cap)| {
                let used_today = used.iter().find(|(t, _)| t == tool).map(|(_, n)| *n).unwrap_or(0);
                Quota { tool: tool.to_string(), daily_cap: cap, used_today, remaining: cap.saturating_sub(used_today) }
            })
            .collect()
    }

    /// Counts one publicly visible write of `tool` against today's cap of the user, or
    /// fails with `rate_limited` when the cap is reached. Call it just before sending.
    pub(crate) fn reserve_public_write(&self, user_id: i64, tool: &'static str) -> Result<(), ToolError> {
        let cap = self.state.cfg.public_write_caps.of(tool).unwrap_or(0);
        match self.state.db.public_write_reserve(user_id, tool, &today(), cap) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                let wait = secs_to_reset();
                Err(ToolError {
                    code: ErrorCode::RateLimited,
                    message: if cap == 0 {
                        format!(
                            "Publicly visible writes of {tool} are forbidden on this server (daily cap 0, operator setting PARFUMO_MCP_PUBLIC_WRITE_CAPS). Nothing was sent. `dry_run` still works."
                        )
                    } else {
                        format!(
                            "Daily cap reached: {tool} may send {cap} publicly visible write(s) per user and per day on this server (operator setting PARFUMO_MCP_PUBLIC_WRITE_CAPS), and {cap} were sent today. Nothing was sent. The count resets at 00:00 UTC, in {wait} seconds; `dry_run` still works. Do not retry before then."
                        )
                    },
                    retry_after_secs: Some(wait),
                })
            }
            Err(e) => Err(ToolError::new(ErrorCode::UpstreamError, format!("The daily cap of {tool} could not be checked ({e}): nothing was sent."))),
        }
    }

    /// Sends the publicly visible writes of one tool call: one unit of today's cap, then
    /// the requests. The unit is given back only when the session was lost before anything
    /// could be sent (the call is then replayed after re-login and counts once).
    pub(crate) async fn send_public(&self, c: &Clients, tool: &'static str, calls: &[WriteCall]) -> Result<Vec<String>, ToolError> {
        self.reserve_public_write(c.user_id, tool)?;
        let mut bodies = Vec::with_capacity(calls.len());
        for (i, call) in calls.iter().enumerate() {
            match c.send(call).await {
                Ok(body) => bodies.push(body),
                Err(e) => {
                    let e = self.ce(e);
                    if i == 0 && e.code == ErrorCode::ReauthRequired {
                        let _ = self.state.db.public_write_release(c.user_id, tool, &today());
                    }
                    return Err(e);
                }
            }
        }
        Ok(bodies)
    }
}
