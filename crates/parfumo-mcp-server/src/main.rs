//! parfumo-mcp-server: remote MCP server (Streamable HTTP) for Parfumo: 48 tools, OAuth 2.1 and
//! static bearer authentication, per-user Parfumo sessions, audit log and daily canary.
//! The server lives in the library of this crate (`lib.rs`); the tools live in
//! `parfumo-mcp-core`, shared with the local stdio binary `parfumo-mcp`.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use parfumo_mcp_core::audit::{self, AuditOptions};
use parfumo_mcp_core::{config, config::Config, crypto, db::Db};
use parfumo_mcp_server::{canary, config::ServerConfig, server};

#[derive(Parser)]
#[command(name = "parfumo-mcp-server", version, about = "Remote MCP server for Parfumo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (/mcp, /healthz, OAuth, /link).
    Serve,
    /// Manage the users of this server (there is no open sign-up).
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Show the audit log of tool calls (kept 90 days), latest first.
    Audit {
        /// Only this user.
        #[arg(long)]
        user: Option<String>,
        /// Only this tool, for example parfumo_send_message.
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
    /// Read the live site once and report every page or route whose structure changed.
    /// The server runs this by itself once a day; exits 1 unless everything is `ok`.
    Canary {
        /// Server user whose linked Parfumo account is read too (default:
        /// PARFUMO_MCP_CANARY_USER). Without one, only public pages are read.
        #[arg(long)]
        user: Option<String>,
        /// Read public pages only, even when a canary user is configured.
        #[arg(long)]
        public_only: bool,
        /// Show the last stored report instead of running a new one.
        #[arg(long)]
        last: bool,
        /// Also post the alert to PARFUMO_MCP_CANARY_WEBHOOK when the result is not `ok`.
        #[arg(long)]
        notify: bool,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a new random master key (base64, 32 bytes) for PARFUMO_MCP_MASTER_KEY.
    Keygen,
    /// Exit 0 when the local server answers on /healthz (used by the Docker health check).
    Healthcheck,
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a user and print, once, their access key.
    Add { name: String },
    /// List users and the state of their Parfumo link.
    List,
    /// Delete a user with all secrets, sessions and tokens.
    Remove { name: String },
    /// Replace the access key, print the new one once, and revoke the user's OAuth sessions.
    RotateKey { name: String },
}

fn valid_user_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn user_command(db: &Db, command: UserCommand) -> Result<(), String> {
    match command {
        UserCommand::Add { name } => {
            if !valid_user_name(&name) {
                return Err("user name: 1 to 32 characters among letters, digits, '-', '_' and '.'".into());
            }
            let key = crypto::new_secret("pmcp_");
            db.add_user(&name, &crypto::token_hash(&key)).map_err(|e| e.to_string())?;
            println!("User created: {name}");
            println!("Access key: {key}");
            println!("It is shown only now; the database keeps a hash. Hand it over through a safe channel.");
            println!("The user pastes it on the /link page, on the login page of an OAuth client, or as `Authorization: Bearer <key>`.");
        }
        UserCommand::List => {
            let users = db.list_users().map_err(|e| e.to_string())?;
            if users.is_empty() {
                println!("No user. Create one with: parfumo-mcp-server user add <name>");
            }
            for u in users {
                let link = match (u.link_mode, u.link_status) {
                    (Some(m), Some(s)) => format!("parfumo link: {m} ({s})"),
                    _ => "parfumo link: none".to_string(),
                };
                println!("{:>4}  {:<32}  created_at={}  {link}", u.id, u.name, u.created_at);
            }
        }
        UserCommand::Remove { name } => {
            if !db.remove_user(&name).map_err(|e| e.to_string())? {
                return Err(format!("no user named `{name}`"));
            }
            println!("User removed: {name} (secrets, sessions and tokens erased)");
        }
        UserCommand::RotateKey { name } => {
            let key = crypto::new_secret("pmcp_");
            if !db.set_user_token(&name, &crypto::token_hash(&key)).map_err(|e| e.to_string())? {
                return Err(format!("no user named `{name}`"));
            }
            println!("New access key for {name}: {key}");
            println!("The old key is refused from now on and the user's OAuth sessions are revoked.");
        }
    }
    Ok(())
}

fn healthcheck(cfg: &ServerConfig) -> Result<(), String> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cfg.bind.port()));
    let timeout = std::time::Duration::from_secs(3);
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).map_err(|e| e.to_string())?;
    let head = String::from_utf8_lossy(&buf[..n]);
    if head.starts_with("HTTP/1.1 200") { Ok(()) } else { Err(format!("unhealthy: {}", head.lines().next().unwrap_or(""))) }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    if matches!(cli.command, Command::Keygen) {
        println!("{}", crypto::MasterKey::generate_base64());
        return Ok(());
    }
    let (cfg, http) = (Config::from_env()?, ServerConfig::from_env()?);
    match cli.command {
        Command::Keygen => Ok(()),
        Command::Healthcheck => healthcheck(&http),
        Command::User { command } => user_command(&Db::open(&cfg.db_path).map_err(|e| e.to_string())?, command),
        Command::Audit { user, tool, since, writes, errors, stats, json, limit } => {
            let db = Db::open(&cfg.db_path).map_err(|e| e.to_string())?;
            for line in audit::report(&db, &AuditOptions { user, tool, since, writes, errors, stats, json, limit })? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Canary { user, public_only, last, notify, json } => {
            let db = Db::open(&cfg.db_path).map_err(|e| e.to_string())?;
            let print = |report: &canary::Report| {
                if json {
                    println!("{}", serde_json::to_string_pretty(report).unwrap_or_default());
                } else {
                    print!("{}", report.text());
                }
            };
            let verdict = |report: &canary::Report| if report.status == "ok" { Ok(()) } else { Err(report.summary()) };
            if last {
                let report = canary::last(&db).ok_or("no canary run is stored yet")?;
                print(&report);
                return verdict(&report);
            }
            let name = if public_only { None } else { user.or_else(|| http.canary_user.clone()) };
            let user = match &name {
                Some(n) => Some(db.user_by_name(n).map_err(|e| e.to_string())?.ok_or_else(|| format!("no user named `{n}`"))?),
                None => None,
            };
            // The master key is only needed to open the linked account of the canary user.
            let key = if user.is_some() { config::master_key_from_env()? } else { crypto::MasterKey::random() };
            let webhook = http.canary_webhook.clone().filter(|_| notify);
            let state = server::state(cfg, http, db, key);
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
            let report = runtime.block_on(async {
                let report = canary::run(&state, user.as_ref()).await;
                if let Some(url) = webhook.filter(|_| report.status != "ok")
                    && let Err(e) = canary::notify(&url, &report).await
                {
                    eprintln!("warning: {e}");
                }
                report
            });
            print(&report);
            verdict(&report)
        }
        Command::Serve => {
            let key = config::master_key_from_env()?;
            let db = Db::open(&cfg.db_path).map_err(|e| e.to_string())?;
            tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?.block_on(server::serve(cfg, http, db, key))
        }
    }
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
