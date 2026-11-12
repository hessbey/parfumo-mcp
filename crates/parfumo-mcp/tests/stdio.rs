//! End-to-end tests of the stdio binary: the real executable, driven over its pipes with
//! JSON-RPC lines, against a local fake standing in for parfumo.com. No network access.

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use serde_json::{Value, json};

const NICK: &str = "testuser";

/// Fake Parfumo: a login form that accepts one password, a session cookie `pf_auth`, the
/// logged-in probe page, and plain public pages.
#[derive(Clone, Default)]
struct Site {
    password: Arc<Mutex<String>>,
    /// Value of `pf_auth` that is a live session right now.
    live: Arc<Mutex<String>>,
    logins: Arc<AtomicU32>,
    /// `(path, Cookie header)` of every request received.
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

async fn site_handler(State(site): State<Site>, uri: Uri, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    site.seen.lock().unwrap().push((uri.path().to_string(), cookie.clone()));
    let redirect = |to: &str| Response::builder().status(StatusCode::FOUND).header(header::LOCATION, to.to_string());
    match uri.path() {
        "/api/account/login" => {
            let n = site.logins.fetch_add(1, Ordering::SeqCst) + 1;
            let form: std::collections::HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
            if form.get("password").map(String::as_str) == Some(site.password.lock().unwrap().as_str()) {
                let session = format!("session{n}");
                *site.live.lock().unwrap() = session.clone();
                redirect("/").header(header::SET_COOKIE, format!("pf_auth={session}; Path=/")).body(Body::empty()).unwrap()
            } else {
                redirect("/account/login_error").body(Body::empty()).unwrap()
            }
        }
        "/account/notifications" => {
            let live = site.live.lock().unwrap().clone();
            if !live.is_empty() && cookie.contains(&format!("pf_auth={live}")) {
                Html(format!("<html><head><title>Notifications</title></head><body><span class=\"nick_name\">{NICK} <i></i></span>ok</body></html>"))
                    .into_response()
            } else {
                redirect("/action/dologin").body(Body::empty()).unwrap()
            }
        }
        p => {
            Html(format!("<html><head><title>Mock page {p}</title></head><body><main><h1>Mock page</h1><p>Hello from {p}.</p></main></body></html>"))
                .into_response()
        }
    }
}

/// Starts the fake site on its own thread; it lives until the test process ends.
fn spawn_site() -> (String, Site) {
    let site = Site::default();
    let app = Router::new().fallback(site_handler).with_state(site.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            let _ = axum::serve(listener, app).await;
        });
    });
    (format!("http://{}", rx.recv().unwrap()), site)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "parfumo-stdio-{tag}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn command(base: &str, db: &Path, env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_parfumo-mcp"));
    // Nothing of the developer's own environment reaches the binary, except what Windows
    // itself needs to start a process that opens sockets.
    cmd.env_clear();
    for system in ["SystemRoot", "SystemDrive", "windir", "TEMP", "TMP"] {
        if let Ok(value) = std::env::var(system) {
            cmd.env(system, value);
        }
    }
    cmd.env("PARFUMO_BASE_URL", base).env("PARFUMO_MCP_DB", db).env("PARFUMO_MCP_USER_JITTER_MS", "0").envs(env.iter().copied());
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd
}

/// One run of the binary, as an MCP client sees it.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    /// Every line the binary wrote on standard output.
    stdout: Vec<String>,
    next_id: u64,
}

impl Server {
    fn start(base: &str, db: &Path, env: &[(&str, &str)]) -> Self {
        let mut child = command(base, db, env).spawn().unwrap();
        let stdin = child.stdin.take();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Self { child, stdin, lines, stdout: Vec::new(), next_id: 0 };
        let init = server.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "stdio-test", "version": "0"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "parfumo-mcp", "{init}");
        assert!(init["result"]["capabilities"]["tools"].is_object(), "{init}");
        server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| panic!("no answer to {method}"));
            self.stdout.push(line.clone());
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("standard output must carry JSON-RPC only, got: {line}"));
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        let answer = self.request("tools/call", json!({"name": tool, "arguments": args}));
        assert!(answer["error"].is_null(), "{tool}: {answer}");
        answer["result"].clone()
    }

    /// Closes standard input (what a client does when it is done) and returns
    /// `(exit status, standard output, standard error)`.
    fn stop(mut self) -> (std::process::ExitStatus, String, String) {
        drop(self.stdin.take());
        let output = self.child.wait_with_output().unwrap();
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(5)) {
            self.stdout.push(line);
        }
        (output.status, self.stdout.join("\n"), String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn without_an_account_the_same_tools_are_listed_and_public_ones_work() {
    let (base, site) = spawn_site();
    let dir = temp_dir("public");
    let mut server = Server::start(&base, &dir.join("p.db"), &[]);

    // Exactly the tools of the HTTP server, with their schemas and annotations.
    let listed = server.request("tools/list", json!({}));
    let mut names: Vec<String> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    let mut expected: Vec<String> = parfumo_mcp_core::tools::ParfumoTools::all_tools().list_all().iter().map(|t| t.name.to_string()).collect();
    expected.sort();
    assert_eq!(names, expected);
    assert_eq!(names.len(), 48);
    for tool in listed["result"]["tools"].as_array().unwrap() {
        assert!(tool["annotations"].is_object() && tool["outputSchema"].is_object() && tool["inputSchema"].is_object(), "{tool}");
    }

    // A public tool reads the site, without any cookie.
    let page = server.call("parfumo_fetch_page", json!({"url": "/rules"}));
    assert_ne!(page["isError"], true, "{page}");
    assert!(page["structuredContent"]["untrusted_page_content"]["text"].as_str().unwrap().contains("Hello from /rules"), "{page}");

    // No account: the status says how to give one, and an account tool answers not_linked.
    let status = server.call("parfumo_session_status", json!({}));
    assert_eq!(status["structuredContent"]["linked"], false);
    assert_eq!(status["structuredContent"]["session"], "not_linked");
    assert!(status["structuredContent"].get("link_url").is_none(), "the stdio server has no /link page: {status}");
    assert!(text(&status).contains("PARFUMO_USER") && !text(&status).contains("/link"), "{}", text(&status));
    let lists = server.call("parfumo_list_collections", json!({}));
    assert_eq!(lists["isError"], true);
    assert!(
        text(&lists).starts_with("not_linked: ") && text(&lists).contains("PARFUMO_COOKIE") && !text(&lists).contains("/link"),
        "{}",
        text(&lists)
    );

    assert_eq!(
        status["structuredContent"]["public_writes_enabled"], false,
        "publicly visible writes stay dry runs by default, as on the HTTP server"
    );

    let (exit, _, stderr) = server.stop();
    assert!(exit.success(), "closing standard input ends the server cleanly: {stderr}");
    assert!(stderr.contains("parfumo-mcp ready"), "logs go to standard error: {stderr}");
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen, [("/rules".to_string(), String::new())], "one public request, no cookie, no login");

    // The local files are private to their owner.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in ["p.db", "p.db.key"] {
            assert_eq!(std::fs::metadata(dir.join(file)).unwrap().permissions().mode() & 0o777, 0o600, "{file}");
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn credentials_of_the_environment_log_in_once_and_the_session_outlives_a_restart() {
    const PASSWORD: &str = "s3cr3t-parfumo-pass";
    let (base, site) = spawn_site();
    *site.password.lock().unwrap() = PASSWORD.into();
    let dir = temp_dir("credentials");
    let db = dir.join("p.db");
    let env = [("PARFUMO_USER", "someone@example.org"), ("PARFUMO_PASSWORD", PASSWORD)];

    // Starting sends nothing; the first account tool logs in.
    let mut server = Server::start(&base, &db, &env);
    assert!(site.seen.lock().unwrap().is_empty(), "no request at start");
    let status = server.call("parfumo_session_status", json!({}));
    assert_eq!(status["structuredContent"]["session"], "valid", "{status}");
    assert_eq!(status["structuredContent"]["nick"], NICK);
    assert_eq!(status["structuredContent"]["mode"], "credentials");
    assert_eq!(site.logins.load(Ordering::SeqCst), 1);
    let (exit, stdout, stderr) = server.stop();
    assert!(exit.success());
    for (name, output) in [("standard output", &stdout), ("standard error", &stderr)] {
        assert!(!output.contains(PASSWORD) && !output.contains("session1"), "a secret leaked on {name}");
    }
    // Neither the password nor the session cookie is stored in clear.
    for file in std::fs::read_dir(&dir).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        assert!(!contains(&bytes, PASSWORD) && !contains(&bytes, "session1"), "a secret is stored in clear");
    }

    // Second run, same account: the stored session is reused, no new login.
    let mut server = Server::start(&base, &db, &env);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "valid");
    assert_eq!(site.logins.load(Ordering::SeqCst), 1, "a restart must not log in again");
    server.stop();

    // The session expired on Parfumo's side: the next run logs in again by itself.
    *site.live.lock().unwrap() = "expired-elsewhere".into();
    let mut server = Server::start(&base, &db, &env);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "valid");
    assert_eq!(site.logins.load(Ordering::SeqCst), 2);
    server.stop();

    // Password changed on Parfumo, not in the environment: one refused attempt, never replayed,
    // neither by the next call nor by the next run.
    *site.live.lock().unwrap() = "expired-again".into();
    *site.password.lock().unwrap() = "changed-on-parfumo".into();
    let mut server = Server::start(&base, &db, &env);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "reauth_required");
    let lists = server.call("parfumo_list_collections", json!({}));
    assert!(text(&lists).starts_with("reauth_required: ") && text(&lists).contains("PARFUMO_PASSWORD"), "{}", text(&lists));
    server.stop();
    let mut server = Server::start(&base, &db, &env);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "reauth_required");
    server.stop();
    assert_eq!(site.logins.load(Ordering::SeqCst), 3, "refused credentials are not sent again");

    // The environment is corrected: the new password is tried, once.
    let mut server = Server::start(&base, &db, &[("PARFUMO_USER", "someone@example.org"), ("PARFUMO_PASSWORD", "changed-on-parfumo")]);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "valid");
    assert_eq!(site.logins.load(Ordering::SeqCst), 4);
    server.stop();

    // The account is removed from the environment: the stored one is erased.
    let mut server = Server::start(&base, &db, &[]);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["linked"], false);
    server.stop();
    assert_eq!(site.logins.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_cookie_of_the_environment_is_used_without_any_login() {
    let (base, site) = spawn_site();
    *site.live.lock().unwrap() = "browser-session".into();
    let dir = temp_dir("cookie");
    let db = dir.join("p.db");

    let mut server = Server::start(&base, &db, &[("PARFUMO_COOKIE", "pf_auth=browser-session")]);
    let status = server.call("parfumo_session_status", json!({}));
    assert_eq!(status["structuredContent"]["session"], "valid", "{status}");
    assert_eq!(status["structuredContent"]["mode"], "cookie");
    assert_eq!(status["structuredContent"]["nick"], NICK);
    let (_, stdout, stderr) = server.stop();
    assert!(!stdout.contains("browser-session") && !stderr.contains("browser-session"));

    // The browser logged out: no password to fall back on, the cookie must be replaced.
    *site.live.lock().unwrap() = "another-session".into();
    let mut server = Server::start(&base, &db, &[("PARFUMO_COOKIE", "pf_auth=browser-session")]);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "reauth_required");
    server.stop();
    let mut server = Server::start(&base, &db, &[("PARFUMO_COOKIE", "pf_auth=another-session")]);
    assert_eq!(server.call("parfumo_session_status", json!({}))["structuredContent"]["session"], "valid");
    server.stop();

    assert_eq!(site.logins.load(Ordering::SeqCst), 0, "cookie mode never logs in");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_ambiguous_environment_is_refused_at_start() {
    let dir = temp_dir("env");
    let db = dir.join("p.db");
    let cases: [(&[(&str, &str)], &str); 4] = [
        (&[("PARFUMO_USER", "someone")], "PARFUMO_PASSWORD"),
        (&[("PARFUMO_PASSWORD", "s3cr3t-value")], "PARFUMO_USER"),
        (&[("PARFUMO_USER", "someone"), ("PARFUMO_PASSWORD", "s3cr3t-value"), ("PARFUMO_COOKIE", "pf_auth=s3cr3t-value")], "not both"),
        (&[("PARFUMO_COOKIE", "s3cr3t-value")], "PARFUMO_COOKIE"),
    ];
    for (env, hint) in cases {
        let output = command("http://127.0.0.1:9", &db, env).stdin(Stdio::null()).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{env:?} must be refused");
        assert!(stderr.contains(hint), "{env:?}: {stderr}");
        assert!(output.stdout.is_empty(), "nothing but the protocol on standard output");
        assert!(!stderr.contains("s3cr3t-value"), "the error must not echo a secret: {stderr}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_audit_subcommand_reads_the_local_log() {
    let (base, _site) = spawn_site();
    let dir = temp_dir("audit");
    let db = dir.join("a.db");

    // Before the first run there is nothing to read, and asking creates nothing.
    let none = command(&base, &db, &[]).arg("audit").output().unwrap();
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("no local database"), "{none:?}");
    assert!(!db.exists(), "reading the audit log never creates the database");

    let mut server = Server::start(&base, &db, &[]);
    let page = server.call("parfumo_fetch_page", json!({"url": "/rules"}));
    assert_ne!(page["isError"], true, "{page}");
    let lists = server.call("parfumo_list_collections", json!({}));
    assert_eq!(lists["isError"], true);
    let (exit, _, _) = server.stop();
    assert!(exit.success());

    let audit = |args: &[&str]| {
        let out = command(&base, &db, &[]).arg("audit").args(args).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    // Latest first, one line per call, under the single local user.
    let all = audit(&[]);
    let lines: Vec<&str> = all.lines().collect();
    assert_eq!(lines.len(), 2, "{all}");
    assert!(lines[0].contains("\tlocal\tparfumo_list_collections\t") && lines[1].contains("\tlocal\tparfumo_fetch_page\t"), "{all}");
    assert!(lines.iter().all(|l| l.contains("writes=0")), "{all}");

    let errors = audit(&["--errors"]);
    assert!(errors.lines().count() == 1 && errors.contains("parfumo_list_collections"), "{errors}");
    assert_eq!(audit(&["--writes"]), "", "nothing was written to Parfumo");
    assert_eq!(audit(&["--tool", "parfumo_fetch_page"]).lines().count(), 1);

    let stats = audit(&["--stats", "--json"]);
    let rows: Vec<Value> = stats.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.len(), 2, "{stats}");
    let fetch = rows.iter().find(|r| r["tool"] == "parfumo_fetch_page").unwrap();
    assert_eq!((fetch["calls"].as_i64(), fetch["errors"].as_i64(), fetch["writes"].as_i64()), (Some(1), Some(0), Some(0)), "{stats}");

    let bad = command(&base, &db, &[]).args(["audit", "--since", "soon"]).output().unwrap();
    assert!(!bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains("--since"), "{bad:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}
