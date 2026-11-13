//! SQLite storage: users, encrypted Parfumo secrets and sessions, OAuth state, cache, audit.
//!
//! Every query touching per-user data takes the `user_id` as a mandatory parameter: there is
//! no API that reads a secret, a session or a token without naming its owner.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};

pub const AUDIT_RETENTION_DAYS: i64 = 90;
/// A dynamically registered OAuth client without any live token is forgotten after this.
pub const CLIENT_IDLE_DAYS: i64 = 30;
/// Upper bound of dynamically registered OAuth clients kept at once.
pub const MAX_OAUTH_CLIENTS: i64 = 500;

/// What became of a refresh token presented to the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshTake {
    /// Valid and now consumed: issue the next pair in the same family.
    Valid { client_id: String, user_id: i64, family: String },
    /// Already exchanged once: a copy is in the wrong hands. The whole family is revoked.
    Reused,
    /// Unknown or expired.
    Invalid,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    token_hash    TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL
);
-- One Parfumo link per user. `secret` and `cookies` are AES-256-GCM blobs.
CREATE TABLE IF NOT EXISTS parfumo_links (
    user_id       INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    mode          TEXT NOT NULL CHECK (mode IN ('credentials', 'cookie')),
    secret        BLOB,
    cookies       BLOB,
    status        TEXT NOT NULL,
    linked_at     INTEGER NOT NULL,
    last_login_at INTEGER,
    -- Changes each time an account is linked: a process that kept a session in memory
    -- sees that another process linked a different account.
    rev           TEXT NOT NULL DEFAULT '',
    -- Stdio server: the account came from the environment of the process.
    from_env      INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS oauth_clients (
    client_id     TEXT PRIMARY KEY,
    client_name   TEXT,
    redirect_uris TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri   TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    expires_at     INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('access', 'refresh')),
    client_id  TEXT NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
-- Refresh tokens already exchanged: presenting one again reveals a stolen token and
-- revokes every token descended from the same authorization (its family).
CREATE TABLE IF NOT EXISTS oauth_refresh_used (
    token_hash TEXT PRIMARY KEY,
    family     TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS login_failures (
    subject TEXT NOT NULL,
    ts      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS login_failures_subject ON login_failures(subject, ts);
-- Public Parfumo pages only (no account data), shared by all users.
CREATE TABLE IF NOT EXISTS cache (
    key        TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS audit (
    id          INTEGER PRIMARY KEY,
    ts          INTEGER NOT NULL,
    user_id     INTEGER NOT NULL,
    user_name   TEXT NOT NULL,
    tool        TEXT NOT NULL,
    target      TEXT NOT NULL,
    result      TEXT NOT NULL,
    duration_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS audit_ts ON audit(ts);
-- Daily canary runs against the live site (kept as long as the audit log).
CREATE TABLE IF NOT EXISTS canary_runs (
    id     INTEGER PRIMARY KEY,
    ts     INTEGER NOT NULL,
    status TEXT NOT NULL,
    report TEXT NOT NULL
);
-- Publicly visible writes sent, per user, tool and UTC day (daily caps).
CREATE TABLE IF NOT EXISTS public_write_counts (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tool    TEXT NOT NULL,
    day     TEXT NOT NULL,
    count   INTEGER NOT NULL,
    PRIMARY KEY (user_id, tool, day)
);
"#;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("a user named `{0}` already exists")]
    UserExists(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub link_mode: Option<String>,
    pub link_status: Option<String>,
}

#[derive(Clone)]
pub struct LinkRow {
    pub mode: String,
    pub secret: Option<Vec<u8>>,
    pub cookies: Option<Vec<u8>>,
    pub status: String,
    pub linked_at: i64,
    pub last_login_at: Option<i64>,
    /// Changes each time an account is linked.
    pub rev: String,
    /// Stdio server: the account came from the environment of the process.
    pub from_env: bool,
}

#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthCode {
    pub client_id: String,
    pub user_id: i64,
    pub redirect_uri: String,
    pub code_challenge: String,
}

#[derive(Debug, Clone)]
pub struct AuditRow {
    pub ts: i64,
    pub user_name: String,
    pub tool: String,
    pub target: String,
    pub result: String,
    pub duration_ms: i64,
    /// Write requests the call really sent to Parfumo (0: a read, a dry run or a refusal).
    pub writes: i64,
}

/// Which audit entries to list. Every field narrows the selection.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub user: Option<String>,
    pub tool: Option<String>,
    /// Only entries at or after this Unix time.
    pub since: Option<i64>,
    /// Only calls that really sent a write to Parfumo.
    pub writes_only: bool,
    /// Only calls that ended in an error.
    pub errors_only: bool,
}

/// Counters of one tool over a period, computed from the audit log.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolStats {
    pub tool: String,
    pub calls: i64,
    pub errors: i64,
    /// Write requests really sent to Parfumo.
    pub writes: i64,
    pub avg_ms: f64,
    pub max_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryRow {
    pub ts: i64,
    pub status: String,
    /// JSON report of the run.
    pub report: String,
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, DbError> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        // Databases created before jalon 5 have no `writes` column in the audit log.
        let has_writes = conn.prepare("SELECT 1 FROM pragma_table_info('audit') WHERE name = 'writes'")?.exists([])?;
        if !has_writes {
            conn.execute_batch("ALTER TABLE audit ADD COLUMN writes INTEGER NOT NULL DEFAULT 0")?;
        }
        let has_family = conn.prepare("SELECT 1 FROM pragma_table_info('oauth_tokens') WHERE name = 'family'")?.exists([])?;
        if !has_family {
            conn.execute_batch("ALTER TABLE oauth_tokens ADD COLUMN family TEXT")?;
        }
        // Databases created before version 0.2 know neither column of the link.
        let has_rev = conn.prepare("SELECT 1 FROM pragma_table_info('parfumo_links') WHERE name = 'rev'")?.exists([])?;
        if !has_rev {
            conn.execute_batch(
                "ALTER TABLE parfumo_links ADD COLUMN rev TEXT NOT NULL DEFAULT '';
                 ALTER TABLE parfumo_links ADD COLUMN from_env INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn ping(&self) -> bool {
        self.conn().query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).is_ok()
    }

    /// Housekeeping: expired cache/OAuth rows, old login failures, audit older than 90 days.
    pub fn purge(&self) -> Result<(), DbError> {
        let c = self.conn();
        let t = now();
        c.execute("DELETE FROM cache WHERE expires_at <= ?1", [t])?;
        c.execute("DELETE FROM oauth_codes WHERE expires_at <= ?1", [t])?;
        c.execute("DELETE FROM oauth_tokens WHERE expires_at <= ?1", [t])?;
        c.execute("DELETE FROM oauth_refresh_used WHERE expires_at <= ?1", [t])?;
        // Dynamically registered clients nobody holds a token for any more.
        c.execute(
            "DELETE FROM oauth_clients WHERE created_at <= ?1 AND client_id NOT IN (SELECT client_id FROM oauth_tokens) \
             AND client_id NOT IN (SELECT client_id FROM oauth_codes)",
            [t - CLIENT_IDLE_DAYS * 86400],
        )?;
        c.execute("DELETE FROM login_failures WHERE ts <= ?1", [t - 86400])?;
        c.execute("DELETE FROM audit WHERE ts <= ?1", [t - AUDIT_RETENTION_DAYS * 86400])?;
        c.execute("DELETE FROM canary_runs WHERE ts <= ?1", [t - AUDIT_RETENTION_DAYS * 86400])?;
        c.execute("DELETE FROM public_write_counts WHERE day < date(?1, 'unixepoch')", [t - AUDIT_RETENTION_DAYS * 86400])?;
        Ok(())
    }

    // ---- users -------------------------------------------------------------------------

    /// Creates a user known by the hash of their access key (the static bearer token).
    pub fn add_user(&self, name: &str, token_hash: &str) -> Result<i64, DbError> {
        let c = self.conn();
        match c.execute("INSERT INTO users (name, token_hash, created_at) VALUES (?1, ?2, ?3)", params![name, token_hash, now()]) {
            Ok(_) => Ok(c.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(DbError::UserExists(name.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_users(&self) -> Result<Vec<UserRow>, DbError> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT u.id, u.name, u.created_at, l.mode, l.status FROM users u \
             LEFT JOIN parfumo_links l ON l.user_id = u.id ORDER BY u.id",
        )?;
        let rows = st
            .query_map([], |r| Ok(UserRow { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, link_mode: r.get(3)?, link_status: r.get(4)? }))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Deletes the user and, by cascade, the Parfumo link, secrets, sessions and OAuth tokens.
    pub fn remove_user(&self, name: &str) -> Result<bool, DbError> {
        Ok(self.conn().execute("DELETE FROM users WHERE name = ?1", [name])? > 0)
    }

    /// Replaces the access key of a user. The old key may have signed the user in to OAuth
    /// clients: every OAuth session of that user is revoked with it.
    pub fn set_user_token(&self, name: &str, token_hash: &str) -> Result<bool, DbError> {
        let c = self.conn();
        let n = c.execute("UPDATE users SET token_hash = ?2 WHERE name = ?1", params![name, token_hash])?;
        c.execute("DELETE FROM oauth_tokens WHERE user_id IN (SELECT id FROM users WHERE name = ?1)", [name])?;
        Ok(n > 0)
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>, DbError> {
        Ok(self.conn().query_row("SELECT id, name FROM users WHERE name = ?1", [name], |r| Ok(User { id: r.get(0)?, name: r.get(1)? })).optional()?)
    }

    /// The user an access key belongs to. Unlike [`Db::user_by_bearer_hash`], an OAuth access
    /// token is not accepted: the web pages ask for the key itself.
    pub fn user_by_key_hash(&self, token_hash: &str) -> Result<Option<User>, DbError> {
        Ok(self
            .conn()
            .query_row("SELECT id, name FROM users WHERE token_hash = ?1", [token_hash], |r| Ok(User { id: r.get(0)?, name: r.get(1)? }))
            .optional()?)
    }

    /// Resolves a bearer token (static per-user token, or unexpired OAuth access token).
    pub fn user_by_bearer_hash(&self, token_hash: &str) -> Result<Option<User>, DbError> {
        let c = self.conn();
        let map = |r: &rusqlite::Row<'_>| Ok(User { id: r.get(0)?, name: r.get(1)? });
        if let Some(u) = c.query_row("SELECT id, name FROM users WHERE token_hash = ?1", [token_hash], map).optional()? {
            return Ok(Some(u));
        }
        Ok(c.query_row(
            "SELECT u.id, u.name FROM oauth_tokens t JOIN users u ON u.id = t.user_id \
             WHERE t.token_hash = ?1 AND t.kind = 'access' AND t.expires_at > ?2",
            params![token_hash, now()],
            map,
        )
        .optional()?)
    }

    // ---- login throttling --------------------------------------------------------------

    pub fn record_login_failure(&self, subject: &str) -> Result<(), DbError> {
        self.conn().execute("INSERT INTO login_failures (subject, ts) VALUES (?1, ?2)", params![subject, now()])?;
        Ok(())
    }

    pub fn recent_login_failures(&self, subject: &str, window_secs: i64) -> Result<i64, DbError> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM login_failures WHERE subject = ?1 AND ts > ?2",
            params![subject, now() - window_secs],
            |r| r.get(0),
        )?)
    }

    pub fn clear_login_failures(&self, subject: &str) -> Result<(), DbError> {
        self.conn().execute("DELETE FROM login_failures WHERE subject = ?1", [subject])?;
        Ok(())
    }

    // ---- Parfumo link (per user) -------------------------------------------------------

    pub fn link_get(&self, user_id: i64) -> Result<Option<LinkRow>, DbError> {
        Ok(self
            .conn()
            .query_row(
                "SELECT mode, secret, cookies, status, linked_at, last_login_at, rev, from_env FROM parfumo_links WHERE user_id = ?1",
                [user_id],
                |r| {
                    Ok(LinkRow {
                        mode: r.get(0)?,
                        secret: r.get(1)?,
                        cookies: r.get(2)?,
                        status: r.get(3)?,
                        linked_at: r.get(4)?,
                        last_login_at: r.get(5)?,
                        rev: r.get(6)?,
                        from_env: r.get(7)?,
                    })
                },
            )
            .optional()?)
    }

    /// Stores the link of a user, replacing any earlier one, and returns its new `rev`.
    pub fn link_put(&self, user_id: i64, mode: &str, secret: Option<&[u8]>, cookies: Option<&[u8]>, status: &str) -> Result<String, DbError> {
        let rev = crate::crypto::new_secret("");
        self.conn().execute(
            "INSERT INTO parfumo_links (user_id, mode, secret, cookies, status, linked_at, rev) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(user_id) DO UPDATE SET mode = ?2, secret = ?3, cookies = ?4, status = ?5, linked_at = ?6, rev = ?7, \
             last_login_at = NULL, from_env = 0",
            params![user_id, mode, secret, cookies, status, now(), rev],
        )?;
        Ok(rev)
    }

    /// Stdio server: marks the stored link as the account of the process environment.
    pub fn link_mark_from_env(&self, user_id: i64) -> Result<(), DbError> {
        self.conn().execute("UPDATE parfumo_links SET from_env = 1 WHERE user_id = ?1", [user_id])?;
        Ok(())
    }

    pub fn link_update_session(&self, user_id: i64, cookies: Option<&[u8]>, status: &str, logged_in_now: bool) -> Result<(), DbError> {
        self.conn().execute(
            "UPDATE parfumo_links SET cookies = ?2, status = ?3, \
             last_login_at = CASE WHEN ?4 THEN ?5 ELSE last_login_at END WHERE user_id = ?1",
            params![user_id, cookies, status, logged_in_now, now()],
        )?;
        Ok(())
    }

    /// Unlink: credentials and cookies are erased.
    pub fn link_delete(&self, user_id: i64) -> Result<bool, DbError> {
        Ok(self.conn().execute("DELETE FROM parfumo_links WHERE user_id = ?1", [user_id])? > 0)
    }

    // ---- OAuth -------------------------------------------------------------------------

    pub fn oauth_client_put(&self, client: &OAuthClient) -> Result<(), DbError> {
        self.conn().execute(
            "INSERT OR REPLACE INTO oauth_clients (client_id, client_name, redirect_uris, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![client.client_id, client.client_name, serde_json::to_string(&client.redirect_uris).unwrap_or_default(), now()],
        )?;
        Ok(())
    }

    pub fn oauth_client_get(&self, client_id: &str) -> Result<Option<OAuthClient>, DbError> {
        Ok(self
            .conn()
            .query_row("SELECT client_id, client_name, redirect_uris FROM oauth_clients WHERE client_id = ?1", [client_id], |r| {
                let uris: String = r.get(2)?;
                Ok(OAuthClient { client_id: r.get(0)?, client_name: r.get(1)?, redirect_uris: serde_json::from_str(&uris).unwrap_or_default() })
            })
            .optional()?)
    }

    pub fn oauth_code_put(&self, code_hash: &str, code: &AuthCode, ttl_secs: i64) -> Result<(), DbError> {
        self.conn().execute(
            "INSERT INTO oauth_codes (code_hash, client_id, user_id, redirect_uri, code_challenge, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![code_hash, code.client_id, code.user_id, code.redirect_uri, code.code_challenge, now() + ttl_secs],
        )?;
        Ok(())
    }

    /// Single use: the code is deleted whether or not it is still valid.
    pub fn oauth_code_take(&self, code_hash: &str) -> Result<Option<AuthCode>, DbError> {
        let c = self.conn();
        let row = c
            .query_row(
                "SELECT client_id, user_id, redirect_uri, code_challenge, expires_at FROM oauth_codes WHERE code_hash = ?1",
                [code_hash],
                |r| {
                    Ok((
                        AuthCode { client_id: r.get(0)?, user_id: r.get(1)?, redirect_uri: r.get(2)?, code_challenge: r.get(3)? },
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        c.execute("DELETE FROM oauth_codes WHERE code_hash = ?1", [code_hash])?;
        Ok(row.filter(|(_, exp)| *exp > now()).map(|(code, _)| code))
    }

    pub fn oauth_client_count(&self) -> Result<i64, DbError> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM oauth_clients", [], |r| r.get(0))?)
    }

    /// `family` ties together the tokens descended from one authorization.
    pub fn oauth_token_put(&self, token_hash: &str, kind: &str, client_id: &str, user_id: i64, ttl: Duration, family: &str) -> Result<(), DbError> {
        self.conn().execute(
            "INSERT INTO oauth_tokens (token_hash, kind, client_id, user_id, expires_at, family) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![token_hash, kind, client_id, user_id, now() + ttl.as_secs() as i64, family],
        )?;
        Ok(())
    }

    /// Refresh tokens rotate: taking one consumes it. Presenting a consumed one again
    /// revokes every access and refresh token of its family (RFC 9700, section 4.14.2).
    pub fn oauth_refresh_take(&self, token_hash: &str) -> Result<RefreshTake, DbError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT client_id, user_id, expires_at, family FROM oauth_tokens WHERE token_hash = ?1 AND kind = 'refresh'",
                [token_hash],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?)),
            )
            .optional()?;
        let taken = match row {
            Some((client_id, user_id, expires_at, family)) => {
                tx.execute("DELETE FROM oauth_tokens WHERE token_hash = ?1", [token_hash])?;
                if expires_at > now() {
                    // Tokens issued before families existed start one here.
                    let family = family.unwrap_or_else(|| token_hash.to_string());
                    tx.execute(
                        "INSERT OR REPLACE INTO oauth_refresh_used (token_hash, family, expires_at) VALUES (?1, ?2, ?3)",
                        params![token_hash, family, expires_at],
                    )?;
                    RefreshTake::Valid { client_id, user_id, family }
                } else {
                    RefreshTake::Invalid
                }
            }
            None => match tx
                .query_row("SELECT family FROM oauth_refresh_used WHERE token_hash = ?1", [token_hash], |r| r.get::<_, String>(0))
                .optional()?
            {
                Some(family) => {
                    tx.execute("DELETE FROM oauth_tokens WHERE family = ?1", [&family])?;
                    RefreshTake::Reused
                }
                None => RefreshTake::Invalid,
            },
        };
        tx.commit()?;
        Ok(taken)
    }

    // ---- cache -------------------------------------------------------------------------

    pub fn cache_get(&self, key: &str) -> Option<String> {
        self.conn()
            .query_row("SELECT body FROM cache WHERE key = ?1 AND expires_at > ?2", params![key, now()], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn cache_put(&self, key: &str, body: &str, ttl: Duration) {
        let _ = self
            .conn()
            .execute("INSERT OR REPLACE INTO cache (key, body, expires_at) VALUES (?1, ?2, ?3)", params![key, body, now() + ttl.as_secs() as i64]);
    }

    // ---- audit -------------------------------------------------------------------------

    /// One entry per tool call. `writes` is the number of write requests really sent to
    /// Parfumo. The log never holds a text, a password, a cookie or a token.
    pub fn audit(&self, user: &User, tool: &str, target: &str, result: &str, duration_ms: u64, writes: u32) {
        let done = self.conn().execute(
            "INSERT INTO audit (ts, user_id, user_name, tool, target, result, duration_ms, writes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![now(), user.id, user.name, tool, target, result, duration_ms as i64, writes],
        );
        if let Err(e) = done {
            // The call itself already happened: say loudly that it left no trace.
            tracing::error!(user = %user.name, tool, result, writes, error = %e, "audit entry could not be written");
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn audit_list(&self, user_name: Option<&str>, limit: u32) -> Result<Vec<AuditRow>, DbError> {
        self.audit_query(&AuditFilter { user: user_name.map(str::to_string), ..AuditFilter::default() }, limit)
    }

    /// Latest entries first.
    pub fn audit_query(&self, f: &AuditFilter, limit: u32) -> Result<Vec<AuditRow>, DbError> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT ts, user_name, tool, target, result, duration_ms, writes FROM audit \
             WHERE (?1 IS NULL OR user_name = ?1) AND (?2 IS NULL OR tool = ?2) AND (?3 IS NULL OR ts >= ?3) \
             AND (?4 = 0 OR writes > 0) AND (?5 = 0 OR result <> 'ok') ORDER BY id DESC LIMIT ?6",
        )?;
        let rows = st
            .query_map(params![f.user, f.tool, f.since, f.writes_only, f.errors_only, limit], |r| {
                Ok(AuditRow {
                    ts: r.get(0)?,
                    user_name: r.get(1)?,
                    tool: r.get(2)?,
                    target: r.get(3)?,
                    result: r.get(4)?,
                    duration_ms: r.get(5)?,
                    writes: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Per-tool counters (calls, errors, writes sent, duration) since a Unix time.
    pub fn audit_stats(&self, user_name: Option<&str>, since: i64) -> Result<Vec<ToolStats>, DbError> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT tool, COUNT(*), SUM(result <> 'ok'), SUM(writes), AVG(duration_ms), MAX(duration_ms) FROM audit \
             WHERE (?1 IS NULL OR user_name = ?1) AND ts >= ?2 GROUP BY tool ORDER BY COUNT(*) DESC, tool",
        )?;
        let rows = st
            .query_map(params![user_name, since], |r| {
                Ok(ToolStats { tool: r.get(0)?, calls: r.get(1)?, errors: r.get(2)?, writes: r.get(3)?, avg_ms: r.get(4)?, max_ms: r.get(5)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- canary ------------------------------------------------------------------------

    pub fn canary_put(&self, status: &str, report: &str) -> Result<(), DbError> {
        self.conn().execute("INSERT INTO canary_runs (ts, status, report) VALUES (?1, ?2, ?3)", params![now(), status, report])?;
        Ok(())
    }

    pub fn canary_last(&self) -> Result<Option<CanaryRow>, DbError> {
        Ok(self
            .conn()
            .query_row("SELECT ts, status, report FROM canary_runs ORDER BY id DESC LIMIT 1", [], |r| {
                Ok(CanaryRow { ts: r.get(0)?, status: r.get(1)?, report: r.get(2)? })
            })
            .optional()?)
    }
}

impl Db {
    // ---- daily caps of the publicly visible writes ----

    /// Counts one publicly visible write of `tool` for this user on `day` (`YYYY-MM-DD`,
    /// UTC) unless the cap is reached. `Ok(Some(n))`: counted, `n` used so far today.
    /// `Ok(None)`: cap reached, nothing counted. Check and increment are one transaction.
    pub fn public_write_reserve(&self, user_id: i64, tool: &str, day: &str, cap: u32) -> Result<Option<u32>, DbError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let used: u32 = tx
            .query_row("SELECT count FROM public_write_counts WHERE user_id = ?1 AND tool = ?2 AND day = ?3", params![user_id, tool, day], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(0);
        if used >= cap {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO public_write_counts (user_id, tool, day, count) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT (user_id, tool, day) DO UPDATE SET count = count + 1",
            params![user_id, tool, day],
        )?;
        tx.commit()?;
        Ok(Some(used + 1))
    }

    /// Gives back a reservation whose write was never sent (session lost before sending).
    pub fn public_write_release(&self, user_id: i64, tool: &str, day: &str) -> Result<(), DbError> {
        self.conn().execute(
            "UPDATE public_write_counts SET count = count - 1 WHERE user_id = ?1 AND tool = ?2 AND day = ?3 AND count > 0",
            params![user_id, tool, day],
        )?;
        Ok(())
    }

    /// Publicly visible writes this user sent on `day`, per tool.
    pub fn public_writes_on(&self, user_id: i64, day: &str) -> Result<Vec<(String, u32)>, DbError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT tool, count FROM public_write_counts WHERE user_id = ?1 AND day = ?2")?;
        let rows = stmt.query_map(params![user_id, day], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

impl parfumo_client::PageCache for Db {
    fn get(&self, key: &str) -> Option<String> {
        self.cache_get(key)
    }
    fn put(&self, key: &str, body: &str, ttl: Duration) {
        self.cache_put(key, body, ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_tokens_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_user("alice", "tok-a").unwrap();
        let b = db.add_user("bob", "tok-b").unwrap();
        assert!(matches!(db.add_user("alice", "tok-c"), Err(DbError::UserExists(_))));
        assert_eq!(db.user_by_bearer_hash("tok-a").unwrap().unwrap().id, a);
        assert_eq!(db.user_by_bearer_hash("tok-b").unwrap().unwrap().id, b);
        assert!(db.user_by_bearer_hash("nope").unwrap().is_none());

        db.link_put(a, "cookie", None, Some(b"blob-a"), "linked").unwrap();
        assert!(db.link_get(b).unwrap().is_none(), "bob must not see alice's link");
        db.oauth_token_put("at", "access", "c", a, Duration::from_secs(60), "fam").unwrap();
        assert_eq!(db.user_by_bearer_hash("at").unwrap().unwrap().id, a);

        assert!(db.remove_user("alice").unwrap());
        assert!(db.link_get(a).unwrap().is_none(), "link erased with the user");
        assert!(db.user_by_bearer_hash("at").unwrap().is_none(), "OAuth tokens erased with the user");
        assert!(db.user_by_bearer_hash("tok-a").unwrap().is_none());
    }

    #[test]
    fn expired_access_token_is_rejected_and_refresh_is_not_a_bearer() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_user("alice", "tok-a").unwrap();
        db.oauth_token_put("old", "access", "c", a, Duration::from_secs(0), "fam").unwrap();
        db.oauth_token_put("rt", "refresh", "c", a, Duration::from_secs(60), "fam").unwrap();
        assert!(db.user_by_bearer_hash("old").unwrap().is_none());
        assert!(db.user_by_bearer_hash("rt").unwrap().is_none());
        assert_eq!(db.oauth_refresh_take("rt").unwrap(), RefreshTake::Valid { client_id: "c".to_string(), user_id: a, family: "fam".to_string() });
        // The legitimate client now holds the next pair of the family.
        db.oauth_token_put("at2", "access", "c", a, Duration::from_secs(60), "fam").unwrap();
        db.oauth_token_put("rt2", "refresh", "c", a, Duration::from_secs(60), "fam").unwrap();
        db.oauth_token_put("other", "access", "c", a, Duration::from_secs(60), "another-family").unwrap();
        assert_eq!(db.oauth_refresh_take("rt").unwrap(), RefreshTake::Reused, "refresh tokens are single use");
        assert!(db.user_by_bearer_hash("at2").unwrap().is_none(), "a replayed refresh token revokes its family");
        assert_eq!(db.oauth_refresh_take("rt2").unwrap(), RefreshTake::Invalid);
        assert!(db.user_by_bearer_hash("other").unwrap().is_some(), "other authorizations of the user are untouched");
        assert_eq!(db.oauth_refresh_take("never-issued").unwrap(), RefreshTake::Invalid);
        db.oauth_token_put("stale", "refresh", "c", a, Duration::from_secs(0), "fam3").unwrap();
        assert_eq!(db.oauth_refresh_take("stale").unwrap(), RefreshTake::Invalid, "expired");

        // Registered clients without any live token are forgotten after a while.
        for (id, age_days) in [("fresh", 0), ("idle", CLIENT_IDLE_DAYS + 1), ("in-use", CLIENT_IDLE_DAYS + 1)] {
            db.oauth_client_put(&OAuthClient { client_id: id.into(), client_name: None, redirect_uris: vec![] }).unwrap();
            db.conn().execute("UPDATE oauth_clients SET created_at = ?2 WHERE client_id = ?1", params![id, now() - age_days * 86400]).unwrap();
        }
        db.oauth_token_put("rt-in-use", "refresh", "in-use", a, Duration::from_secs(60), "fam4").unwrap();
        assert_eq!(db.oauth_client_count().unwrap(), 3);
        db.purge().unwrap();
        assert!(db.oauth_client_get("fresh").unwrap().is_some() && db.oauth_client_get("in-use").unwrap().is_some());
        assert!(db.oauth_client_get("idle").unwrap().is_none());
    }

    #[test]
    fn auth_code_is_single_use() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_user("alice", "tok-a").unwrap();
        let code = AuthCode { client_id: "c".into(), user_id: a, redirect_uri: "http://localhost/cb".into(), code_challenge: "x".into() };
        db.oauth_code_put("h", &code, 60).unwrap();
        assert!(db.oauth_code_take("h").unwrap().is_some());
        assert!(db.oauth_code_take("h").unwrap().is_none());
        db.oauth_code_put("h2", &code, 0).unwrap();
        assert!(db.oauth_code_take("h2").unwrap().is_none(), "expired code");
    }

    #[test]
    fn public_write_counts_are_per_user_tool_and_day() {
        let db = Db::open_in_memory().unwrap();
        let alice = db.add_user("alice", "t1").unwrap();
        let bob = db.add_user("bob", "t2").unwrap();
        assert_eq!(db.public_write_reserve(alice, "parfumo_send_message", "2031-05-05", 2).unwrap(), Some(1));
        assert_eq!(db.public_write_reserve(alice, "parfumo_send_message", "2031-05-05", 2).unwrap(), Some(2));
        assert_eq!(db.public_write_reserve(alice, "parfumo_send_message", "2031-05-05", 2).unwrap(), None, "cap reached");
        assert_eq!(db.public_write_reserve(alice, "parfumo_forum_post", "2031-05-05", 2).unwrap(), Some(1), "another tool has its own count");
        assert_eq!(db.public_write_reserve(bob, "parfumo_send_message", "2031-05-05", 2).unwrap(), Some(1), "another user has their own count");
        assert_eq!(db.public_write_reserve(alice, "parfumo_send_message", "2031-05-06", 2).unwrap(), Some(1), "a new day starts at zero");
        assert_eq!(db.public_write_reserve(alice, "parfumo_wall", "2031-05-05", 0).unwrap(), None, "a cap of 0 forbids");
        db.public_write_release(alice, "parfumo_send_message", "2031-05-05").unwrap();
        let mut counts = db.public_writes_on(alice, "2031-05-05").unwrap();
        counts.sort();
        assert_eq!(counts, [("parfumo_forum_post".to_string(), 1), ("parfumo_send_message".to_string(), 1)]);
        db.public_write_reserve(alice, "parfumo_wall", "2020-01-01", 5).unwrap();
        let today = parfumo_client::parse::iso_date(now());
        db.public_write_reserve(alice, "parfumo_wall", &today, 5).unwrap();
        db.purge().unwrap();
        assert!(db.public_writes_on(alice, "2020-01-01").unwrap().is_empty(), "old days are purged after the retention period");
        assert_eq!(db.public_writes_on(alice, &today).unwrap(), [("parfumo_wall".to_string(), 1)]);
        assert_eq!(db.public_writes_on(alice, "2031-05-05").unwrap().len(), 2, "recent and future days are kept");
        db.remove_user("alice").unwrap();
        assert!(db.public_writes_on(alice, "2031-05-06").unwrap().is_empty(), "counts go with the user");
    }

    #[test]
    fn cache_ttl_and_audit() {
        let db = Db::open_in_memory().unwrap();
        db.cache_put("k", "v", Duration::from_secs(60));
        db.cache_put("gone", "v", Duration::from_secs(0));
        assert_eq!(db.cache_get("k").as_deref(), Some("v"));
        assert_eq!(db.cache_get("gone"), None);
        db.purge().unwrap();
        let u = User { id: 1, name: "alice".into() };
        db.audit(&u, "parfumo_get_perfume", "Dior/Sauvage", "ok", 12, 0);
        db.audit(&User { id: 2, name: "bob".into() }, "parfumo_get_note", "Vanilla", "not_found", 5, 0);
        db.audit(&u, "parfumo_add_to_collection", "Dior/Sauvage -> have", "ok", 30, 1);
        let writes = db.audit_query(&AuditFilter { writes_only: true, ..AuditFilter::default() }, 10).unwrap();
        assert_eq!((writes.len(), writes[0].tool.as_str(), writes[0].writes), (1, "parfumo_add_to_collection", 1));
        let errors = db.audit_query(&AuditFilter { errors_only: true, ..AuditFilter::default() }, 10).unwrap();
        assert_eq!((errors.len(), errors[0].result.as_str()), (1, "not_found"));
        assert_eq!(
            db.audit_query(&AuditFilter { tool: Some("parfumo_get_note".into()), since: Some(now() - 60), ..AuditFilter::default() }, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(db.audit_query(&AuditFilter { since: Some(now() + 60), ..AuditFilter::default() }, 10).unwrap().is_empty());
        let stats = db.audit_stats(None, 0).unwrap();
        let note = stats.iter().find(|s| s.tool == "parfumo_get_note").unwrap();
        assert_eq!((note.calls, note.errors, note.writes, note.max_ms), (1, 1, 0, 5));
        assert_eq!(db.audit_stats(Some("alice"), 0).unwrap().iter().map(|s| s.writes).sum::<i64>(), 1);
        assert_eq!(db.audit_list(None, 10).unwrap().len(), 3);
        db.conn().execute("DELETE FROM audit WHERE tool = 'parfumo_add_to_collection'", []).unwrap();
        assert_eq!(db.audit_list(None, 10).unwrap().len(), 2);
        let only = db.audit_list(Some("alice"), 10).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].tool, "parfumo_get_perfume");
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn a_database_of_version_0_1_keeps_its_link_and_gains_the_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, token_hash TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
             CREATE TABLE parfumo_links (user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE, mode TEXT NOT NULL, secret BLOB, \
             cookies BLOB, status TEXT NOT NULL, linked_at INTEGER NOT NULL, last_login_at INTEGER);
             INSERT INTO users VALUES (1, 'local', 'h', 1);
             INSERT INTO parfumo_links VALUES (1, 'cookie', NULL, x'00', 'linked', 5, NULL);",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        let old = db.link_get(1).unwrap().unwrap();
        assert_eq!((old.mode.as_str(), old.linked_at, old.rev.as_str(), old.from_env), ("cookie", 5, "", false));
        db.link_mark_from_env(1).unwrap();
        assert!(db.link_get(1).unwrap().unwrap().from_env);
        // Linking again gives a new `rev` each time, and forgets where the old account came from.
        let first = db.link_put(1, "credentials", Some(b"s"), None, "linked").unwrap();
        let second = db.link_put(1, "credentials", Some(b"s"), None, "linked").unwrap();
        let link = db.link_get(1).unwrap().unwrap();
        assert!(first != second && link.rev == second && !link.from_env);
        // Opening the same database again must not fail on the existing columns.
        let again = Db::init(std::mem::replace(&mut *db.conn(), Connection::open_in_memory().unwrap())).unwrap();
        assert_eq!(again.link_get(1).unwrap().unwrap().rev, second);
    }

    #[test]
    fn a_database_from_before_jalon_5_gains_the_writes_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE audit (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, user_id INTEGER NOT NULL, user_name TEXT NOT NULL, \
             tool TEXT NOT NULL, target TEXT NOT NULL, result TEXT NOT NULL, duration_ms INTEGER NOT NULL);
             INSERT INTO audit (ts, user_id, user_name, tool, target, result, duration_ms) VALUES (1, 1, 'alice', 'parfumo_get_perfume', 'x', 'ok', 3);",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        db.audit(&User { id: 1, name: "alice".into() }, "parfumo_rate_perfume", "x", "ok", 9, 2);
        let rows = db.audit_list(Some("alice"), 10).unwrap();
        assert_eq!(rows.iter().map(|r| r.writes).collect::<Vec<_>>(), [2, 0], "old entries count no write");
        // Opening the same database again must not fail on the existing column.
        let again = Db::init(std::mem::replace(&mut *db.conn(), Connection::open_in_memory().unwrap())).unwrap();
        assert_eq!(again.audit_list(None, 10).unwrap().len(), 2);
    }

    #[test]
    fn canary_runs_keep_the_latest_and_are_purged_with_the_audit_log() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.canary_last().unwrap(), None);
        db.canary_put("ok", "{}").unwrap();
        db.canary_put("changed", "{\"x\":1}").unwrap();
        let last = db.canary_last().unwrap().unwrap();
        assert_eq!((last.status.as_str(), last.report.as_str()), ("changed", "{\"x\":1}"));
        db.conn().execute("UPDATE canary_runs SET ts = 5", []).unwrap();
        db.purge().unwrap();
        assert_eq!(db.canary_last().unwrap(), None);
    }
}
