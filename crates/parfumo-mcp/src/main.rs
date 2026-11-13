//! parfumo-mcp: local MCP server for Parfumo over stdio.
//!
//! Same 48 tools as the HTTP server `parfumo-mcp-server`, for one person on their own machine:
//! no OAuth, no users. The single Parfumo account is linked on a login page that the
//! binary opens on 127.0.0.1 when it is needed (`login`), or read from the environment
//! (`PARFUMO_USER` + `PARFUMO_PASSWORD`, or `PARFUMO_COOKIE`); without one, the public
//! tools still work. Cache, session cookies, audit log and daily counts live in a local
//! SQLite file.
//!
//! Standard output carries the MCP protocol and nothing else: logs go to standard error.

mod login;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use clap::{Parser, Subcommand};
use parfumo_mcp_core::{
    audit::{self, AuditOptions},
    config::{self, Config},
    crypto::{self, MasterKey},
    db::{Db, User},
    sessions::{self, LocalAccount},
    state::AppState,
    tools::{LinkHow, ParfumoTools},
};
use rmcp::ServiceExt;

/// Name of the single user every call, audit entry and daily count belongs to.
const LOCAL_USER: &str = "local";

#[derive(Parser)]
#[command(
    name = "parfumo-mcp",
    version,
    about = "Local MCP server for Parfumo over stdio (one account, linked with `parfumo-mcp login`)",
    after_help = "Parfumo account: run `parfumo-mcp login`, or let the assistant give you the link of the\n\
        login page the first time a tool needs the account. Without an account, the public tools\n\
        still work.\n\n\
        Environment:\n  \
        PARFUMO_MCP_DB                  local SQLite file (default: user data directory)\n  \
        PARFUMO_MCP_PUBLIC_WRITES       `true` really sends writes other members can see\n  \
        PARFUMO_USER, PARFUMO_PASSWORD  account given by the environment instead of the login page\n  \
        PARFUMO_COOKIE                  the same, with the `pf_auth=…` cookie of a logged-in browser\n\
        Other PARFUMO_MCP_* settings: see the README.\n\n\
        Without a subcommand, the binary serves MCP over standard input and output."
)]
struct Cli {
    /// Without a subcommand: serve MCP over standard input and output.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show the local audit log of tool calls (kept 90 days), latest first.
    Audit {
        /// Only this tool, for example parfumo_log_wear.
        #[arg(long)]
        tool: Option<String>,
        /// Only entries newer than this: a number of minutes, hours or days (90m, 24h, 7d).
        #[arg(long)]
        since: Option<String>,
        /// Only calls that really sent a write to Parfumo (no read, no dry run).
        #[arg(long)]
        writes: bool,
        /// Only calls that ended in an error.
        #[arg(long)]
        errors: bool,
        /// Per-tool counters (calls, errors, writes sent, duration) instead of the entries.
        #[arg(long)]
        stats: bool,
        /// One JSON object per line.
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Link a Parfumo account on a page of this computer, opened in your browser.
    Login {
        /// Only print the address of the page, without opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Erase the Parfumo account stored on this computer.
    Logout,
}

/// The local database file: `PARFUMO_MCP_DB` when set, the default file otherwise. The
/// flag tells whether it is the default one.
fn db_path(var: impl Fn(&str) -> Option<String>) -> Result<(PathBuf, bool), String> {
    match var("PARFUMO_MCP_DB").filter(|v| !v.trim().is_empty()) {
        Some(path) => Ok((PathBuf::from(path), false)),
        None => Ok((default_db_path(var)?, true)),
    }
}

/// `parfumo-mcp audit`: reads the local database, changes nothing, needs no account.
fn audit_command(options: &AuditOptions) -> Result<(), String> {
    let (path, _) = db_path(|name| std::env::var(name).ok())?;
    if !path.exists() {
        return Err(format!("no local database at {}: no tool call has been recorded yet", path.display()));
    }
    let db = Db::open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    for line in audit::report(&db, options)? {
        println!("{line}");
    }
    Ok(())
}

/// Default database file: `parfumo-mcp/parfumo-mcp.db` in the user's data directory.
fn default_db_path(var: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    let get = |name: &str| var(name).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    let base = get("XDG_DATA_HOME")
        .or_else(|| get("LOCALAPPDATA"))
        .or_else(|| get("HOME").map(|home| home.join(".local").join("share")))
        .ok_or("no user data directory found (HOME is not set): set PARFUMO_MCP_DB to the path of the local database file")?;
    Ok(base.join("parfumo-mcp").join("parfumo-mcp.db"))
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

/// Key that encrypts the stored session and credentials. Taken from the environment when
/// given (as on the HTTP server); otherwise from a key file next to the database, created
/// on first use and readable by its owner only.
fn master_key(db_path: &Path, var: impl Fn(&str) -> Option<String>) -> Result<MasterKey, String> {
    if var(config::ENV_MASTER_KEY).is_some() || var(config::ENV_MASTER_KEY_FILE).is_some() {
        return config::master_key_from_env();
    }
    key_file(&PathBuf::from(format!("{}.key", db_path.display())))
}

fn key_file(path: &Path) -> Result<MasterKey, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => MasterKey::from_base64(text.trim())
            .map_err(|e| format!("{}: {e}. Delete this file and the database next to it to start afresh.", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let text = MasterKey::generate_base64();
            // `create_new`: never overwrite a key another process has just written.
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            match options.open(path).and_then(|mut file| file.write_all(text.as_bytes())) {
                Ok(()) => MasterKey::from_base64(&text).map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => key_file(path),
                Err(e) => Err(format!("cannot write {}: {e}", path.display())),
            }
        }
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The single local user, created on first use. It has no usable password or token:
/// nothing authenticates against it.
fn local_user(db: &Db) -> Result<User, String> {
    if let Some(user) = db.user_by_name(LOCAL_USER).map_err(|e| e.to_string())? {
        return Ok(user);
    }
    match db.add_user(LOCAL_USER, &crypto::token_hash(&crypto::new_secret("unused_"))) {
        Ok(_) | Err(parfumo_mcp_core::db::DbError::UserExists(_)) => {}
        Err(e) => return Err(e.to_string()),
    }
    db.user_by_name(LOCAL_USER).map_err(|e| e.to_string())?.ok_or_else(|| "the local user could not be created".to_string())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

async fn serve(tools: ParfumoTools, db: Db) -> Result<(), String> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = db.purge() {
                tracing::warn!(error = %e, "database purge failed");
            }
        }
    });
    let service = tools.serve(rmcp::transport::stdio()).await.map_err(|e| format!("MCP handshake failed: {e}"))?;
    let cancel = service.cancellation_token();
    tokio::spawn(async move {
        shutdown_signal().await;
        cancel.cancel();
    });
    // Ends when the client closes standard input, or on SIGINT / SIGTERM.
    service.waiting().await.map(|_| ()).map_err(|e| e.to_string())
}

/// What every command but `audit` works on: the local database, its single user, and the
/// account the environment gives, if any.
struct Local {
    state: AppState,
    db: Db,
    user: User,
    account: Option<LocalAccount>,
}

fn open_local() -> Result<Local, String> {
    let var = |name: &str| std::env::var(name).ok();
    // Logs on standard error only: standard output is the protocol channel.
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let targets = tracing_subscriber::filter::Targets::new().with_default(tracing::Level::INFO).with_target("rmcp", tracing::Level::WARN);
    tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_ansi(false)).with(targets).init();

    let account = LocalAccount::from_vars(var)?;
    let mut cfg = Config::from_env()?;
    let (path, default_dir) = db_path(var)?;
    cfg.db_path = path;
    if default_dir && let Some(dir) = cfg.db_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        restrict(dir, 0o700);
    }
    // Created private from the start, so that SQLite gives its side files the same mode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(&cfg.db_path);
    }
    let db = Db::open(&cfg.db_path).map_err(|e| format!("cannot open {}: {e}", cfg.db_path.display()))?;
    restrict(&cfg.db_path, 0o600);
    let key = master_key(&cfg.db_path, var)?;
    db.purge().map_err(|e| e.to_string())?;
    let user = local_user(&db)?;
    Ok(Local { state: AppState::new(cfg, db.clone(), key), db, user, account })
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())
}

/// `parfumo-mcp login`: opens the login page and waits until an account is linked. A
/// server that is already running finds the account at its next tool call.
fn login_command(no_browser: bool) -> Result<(), String> {
    let local = open_local()?;
    if local.account.is_some() {
        return Err(format!(
            "the Parfumo account is given by {}, {} or {}: remove these variables to use the login page",
            sessions::ENV_LOCAL_USER,
            sessions::ENV_LOCAL_PASSWORD,
            sessions::ENV_LOCAL_COOKIE
        ));
    }
    runtime()?.block_on(async {
        let page = login::LoginPage::new(local.state.sessions.clone(), local.user.id);
        let url = page.url().ok_or("the login page could not be opened on 127.0.0.1")?;
        println!("Link your Parfumo account on this page, open on this computer for ten minutes:\n\n  {url}\n");
        if !no_browser && !login::open_browser(&url) {
            println!("No browser could be started: open the address above yourself.");
        }
        match page.ended().await {
            login::Ending::Linked => {
                println!("Your Parfumo account is linked.");
                Ok(())
            }
            _ => Err("the login page closed after ten minutes without an account being linked".to_string()),
        }
    })
}

/// `parfumo-mcp logout`: erases the stored account.
fn logout_command() -> Result<(), String> {
    let local = open_local()?;
    let was_linked = local.state.sessions.unlink(local.user.id).map_err(|e| e.to_string())?;
    println!(
        "{}",
        if was_linked {
            "The Parfumo account is unlinked: the credentials and cookies stored on this computer are erased."
        } else {
            "No Parfumo account was linked."
        }
    );
    if local.account.is_some() {
        println!("An account is still given by the PARFUMO_* variables of the environment: remove them too.");
    }
    Ok(())
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Some(Command::Audit { tool, since, writes, errors, stats, json, limit }) => {
            return audit_command(&AuditOptions { user: None, tool, since, writes, errors, stats, json, limit });
        }
        Some(Command::Login { no_browser }) => return login_command(no_browser),
        Some(Command::Logout) => return logout_command(),
        None => {}
    }
    let Local { state, db, user, account } = open_local()?;
    let changed = state.sessions.sync_local_link(user.id, account.as_ref()).map_err(|e| e.to_string())?;
    let linked = state.sessions.link_info(user.id).map_err(|e| e.to_string())?.is_some();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        db = %state.cfg.db_path.display(),
        account = match (&account, linked) {
            (Some(LocalAccount::Credentials(_)), _) => "credentials, from the environment",
            (Some(LocalAccount::Cookie(_)), _) => "cookie, from the environment",
            (None, true) => "linked on the login page",
            (None, false) => "none yet (public tools only; the login page opens when a tool needs the account)",
        },
        account_changed = changed,
        public_writes = state.cfg.public_writes,
        "parfumo-mcp ready"
    );

    // An account of the environment is changed there; otherwise on the login page, which
    // is opened the first time a message tells the user to link an account.
    let link_how = match account {
        Some(_) => LinkHow::Environment,
        None => {
            let page = login::LoginPage::new(state.sessions.clone(), user.id);
            LinkHow::LocalPage(Arc::new(move || page.url()))
        }
    };
    let tools = ParfumoTools::local(state, user, link_how);
    runtime()?.block_on(serve(tools, db))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    #[test]
    fn database_goes_to_the_user_data_directory() {
        assert_eq!(
            default_db_path(vars(&[("XDG_DATA_HOME", "/x/data"), ("HOME", "/home/u")])).unwrap(),
            PathBuf::from("/x/data/parfumo-mcp/parfumo-mcp.db")
        );
        assert_eq!(default_db_path(vars(&[("HOME", "/home/u")])).unwrap(), PathBuf::from("/home/u/.local/share/parfumo-mcp/parfumo-mcp.db"));
        assert!(default_db_path(vars(&[("HOME", " ")])).unwrap_err().contains("PARFUMO_MCP_DB"));
    }

    #[test]
    fn key_file_is_created_once_and_private() {
        let dir = std::env::temp_dir().join(format!("parfumo-stdio-key-{}", crypto::new_secret("")));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("p.db");
        let key = master_key(&db_path, |_| None).unwrap();
        let blob = key.encrypt(b"session", "aad").unwrap();
        // A second start reads the same key: what the first one stored stays readable.
        let again = master_key(&db_path, |_| None).unwrap();
        assert_eq!(again.decrypt(&blob, "aad").unwrap(), b"session");
        let key_path = dir.join("p.db.key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A damaged key file is reported, never silently replaced.
        std::fs::write(&key_path, "not a key").unwrap();
        assert!(master_key(&db_path, |_| None).err().unwrap().contains("p.db.key"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_local_user_is_created_once() {
        let dir = std::env::temp_dir().join(format!("parfumo-stdio-user-{}", crypto::new_secret("")));
        let db = Db::open(&dir.join("p.db")).unwrap();
        let first = local_user(&db).unwrap();
        assert_eq!(local_user(&db).unwrap(), first);
        assert_eq!(first.name, LOCAL_USER);
        // Windows refuses to delete a database file that is still open.
        drop(db);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
