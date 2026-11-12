//! Logged-in exploration tool (jalon 0 bis). One throttled request per invocation,
//! through the production transport (same User-Agent, cookie jar, challenge detection).
//!
//! cargo run -q -p parfumo-client --features explore --example explore -- <command>
//!   login                      one login attempt with PARFUMO_TEST_USER / PARFUMO_TEST_PASSWORD
//!                              read from ./.env (three attempts at most, counted on disk)
//!   import-cookie              imports PARFUMO_TEST_COOKIE (a browser `Cookie:` header) from ./.env
//!                              and probes a protected route
//!   csrf                       refreshes the session csrf_key from /api/alive
//!   req <METHOD> <path> [k=v …] [-H 'Name: value'] [--raw <content-type> <body>] [-o <name>]
//!                              `@csrf` in a value is the stored csrf_key, `@user` the stored pseudo
//!
//! State (cookies, csrf_key, attempt counter, bodies) lives in $PARFUMO_EXPLORE_DIR
//! (default /tmp/parfumo-explore), outside the repository. Cookie values, the csrf_key and
//! the credentials are never printed.

use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parfumo_client::{site::routes, *};

const MAX_LOGIN_ATTEMPTS: u32 = 3;
/// Never requested, whatever the method (cahier des charges, section 6). Account deletion
/// is matched on the exact route, so that `/api/account/delete_iwear` (a
/// wear) or `/api/account/delete_pm` (a message) stay reachable to undo a test.
const FORBIDDEN_EXACT: &[&str] = &["/account/delete", "/api/account/delete", "/account/delete_account", "/api/account/delete_account"];
/// Substrings never requested: password and e-mail changes, purchases.
const FORBIDDEN: &[&str] = &["password", "passwort", "email", "e-mail", "premium", "checkout", "payment"];

fn dir() -> PathBuf {
    let d = PathBuf::from(std::env::var("PARFUMO_EXPLORE_DIR").unwrap_or_else(|_| "/tmp/parfumo-explore".into()));
    fs::create_dir_all(d.join("out")).expect("state dir");
    d
}

fn read(name: &str) -> Option<String> {
    fs::read_to_string(dir().join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn write(name: &str, content: &str) {
    let path = dir().join(name);
    fs::write(&path, content).expect("write state");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

/// `KEY='value'` / `KEY="value"` / `KEY=value` from ./.env, quotes respected.
fn env_value(key: &str) -> Option<String> {
    let text = fs::read_to_string(".env").ok()?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix('=')) else { continue };
        let rest = rest.trim();
        for q in ['\'', '"'] {
            if rest.len() >= 2 && rest.starts_with(q) && rest.ends_with(q) {
                return Some(rest[1..rest.len() - 1].to_string());
            }
        }
        return Some(rest.to_string());
    }
    None
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Cross-process throttle: at least 1 s plus a random delay since the previous invocation.
async fn wait_turn() {
    let last: u128 = read("last_request_ms").and_then(|s| s.parse().ok()).unwrap_or(0);
    let jitter = (now_ms() % 500) + 100;
    let due = last + 1000 + jitter;
    let now = now_ms();
    if due > now {
        tokio::time::sleep(Duration::from_millis((due - now) as u64)).await;
    }
}

fn mask(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if s.len() >= 3 {
            out = out.replace(s.as_str(), "<redacted>");
        }
    }
    out
}

fn show(resp: &ExploreResponse, out_name: &str, secrets: &[String]) {
    println!("status: {}", resp.status);
    for (k, v) in &resp.headers {
        match k.as_str() {
            "set-cookie" => {
                let (nv, attrs) = v.split_once(';').unwrap_or((v, ""));
                let (name, value) = nv.split_once('=').unwrap_or((nv, ""));
                println!("set-cookie: {name}=<{} chars>;{}", value.len(), mask(attrs, secrets));
            }
            "location"
            | "content-type"
            | "content-disposition"
            | "retry-after"
            | "cf-mitigated"
            | "cache-control"
            | "x-ratelimit-limit"
            | "x-ratelimit-remaining"
            | "www-authenticate" => println!("{k}: {}", mask(v, secrets)),
            _ => {}
        }
    }
    let path = dir().join("out").join(out_name);
    fs::write(&path, &resp.body).expect("write body");
    let text = String::from_utf8_lossy(&resp.body);
    let preview: String = text.chars().take(400).collect();
    println!("body: {} bytes -> {}", resp.body.len(), path.display());
    println!("preview: {}", mask(&preview.replace('\n', " "), secrets));
}

fn client() -> ParfumoClient {
    let mut cfg = ClientConfig::default();
    if let Ok(base) = std::env::var("PARFUMO_BASE_URL") {
        cfg.base_url = base;
    }
    let client = ParfumoClient::new(cfg, GlobalLimiter::new(1), None).expect("client");
    if let Some(json) = read("jar.json") {
        let cookies: Vec<StoredCookie> = serde_json::from_str(&json).expect("jar.json");
        client.import_cookies(&cookies);
    }
    client
}

fn save(client: &ParfumoClient) {
    write("jar.json", &serde_json::to_string(&client.export_cookies()).expect("jar"));
    write("last_request_ms", &now_ms().to_string());
}

fn secrets(client: &ParfumoClient) -> Vec<String> {
    let mut s: Vec<String> = client.export_cookies().into_iter().map(|c| c.value).collect();
    s.extend(read("csrf"));
    s.extend(env_value("PARFUMO_TEST_USER"));
    s.extend(env_value("PARFUMO_TEST_PASSWORD"));
    s.extend(read("pseudo"));
    s
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let client = client();
    wait_turn().await;
    match args.first().map(String::as_str).unwrap_or("") {
        "login" => {
            let attempts: u32 = read("login_attempts").and_then(|s| s.parse().ok()).unwrap_or(0);
            if attempts >= MAX_LOGIN_ATTEMPTS {
                eprintln!("REFUSED: {attempts} login attempts already made");
                std::process::exit(3);
            }
            let user = env_value("PARFUMO_TEST_USER").expect("PARFUMO_TEST_USER in .env");
            let pass = env_value("PARFUMO_TEST_PASSWORD").expect("PARFUMO_TEST_PASSWORD in .env");
            write("login_attempts", &(attempts + 1).to_string());
            let form: Vec<(String, String)> =
                [("username", user.as_str()), ("password", pass.as_str()), ("autologin", "checked"), ("login", "Login"), ("redirect", "")]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
            println!("login attempt {} of {MAX_LOGIN_ATTEMPTS}", attempts + 1);
            let base = client.base_url();
            let headers: Vec<(String, String)> = if std::env::var("EXPLORE_LOGIN_ORIGIN").is_ok() {
                vec![("Origin".into(), base.clone()), ("Referer".into(), format!("{base}/action/dologin"))]
            } else {
                Vec::new()
            };
            println!("origin/referer headers sent: {}", !headers.is_empty());
            match client.explore_request("POST", routes::LOGIN, Some(&form), None, &headers).await {
                Ok(resp) => {
                    save(&client);
                    show(&resp, "login_response", &secrets(&client));
                    println!("cookies in jar: {:?}", client.export_cookies().iter().map(|c| c.name.clone()).collect::<Vec<_>>());
                }
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        "import-cookie" => {
            // Fallback when the password login is refused: PARFUMO_TEST_COOKIE in .env holds the
            // `Cookie:` header copied from a browser session of the test account.
            let header = env_value("PARFUMO_TEST_COOKIE").expect("PARFUMO_TEST_COOKIE in .env");
            let count = client.import_cookie_header(&header);
            match client.is_logged_in().await {
                Ok(state) => {
                    save(&client);
                    println!("{count} cookies imported, logged in: {state}");
                }
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        "csrf" => match client.csrf_key().await {
            Ok(key) => {
                write("csrf", &key);
                save(&client);
                println!("csrf_key stored ({} chars)", key.len());
            }
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        },
        "req" => {
            let method = args.get(1).expect("method").to_uppercase();
            let subst = |v: &str| v.replace("@csrf", &read("csrf").unwrap_or_default()).replace("@user", &read("pseudo").unwrap_or_default());
            let path = subst(args.get(2).expect("path"));
            let lower = path.to_lowercase();
            let bare = lower.split(['?', '#']).next().unwrap_or("").trim_end_matches('/').to_string();
            if FORBIDDEN.iter().any(|f| lower.contains(f)) || FORBIDDEN_EXACT.contains(&bare.as_str()) || bare.starts_with("/account/delete/") {
                eprintln!("REFUSED: forbidden route");
                std::process::exit(3);
            }
            let mut form: Vec<(String, String)> = Vec::new();
            let mut headers: Vec<(String, String)> = Vec::new();
            let mut raw: Option<(String, Vec<u8>)> = None;
            let mut out = format!("{}_{}", method.to_lowercase(), path.trim_matches('/').replace(['/', '?', '&', '='], "_"));
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "-H" => {
                        let (k, v) = args[i + 1].split_once(':').expect("header");
                        headers.push((k.trim().to_string(), subst(v.trim())));
                        i += 2;
                    }
                    "--raw" => {
                        raw = Some((args[i + 1].clone(), subst(&args[i + 2]).into_bytes()));
                        i += 3;
                    }
                    "-o" => {
                        out = args[i + 1].clone();
                        i += 2;
                    }
                    kv => {
                        let (k, v) = kv.split_once('=').expect("k=v");
                        form.push((k.to_string(), subst(v)));
                        i += 1;
                    }
                }
            }
            let form_opt = if form.is_empty() || raw.is_some() { None } else { Some(form.as_slice()) };
            match client.explore_request(&method, &path, form_opt, raw, &headers).await {
                Ok(resp) => {
                    save(&client);
                    show(&resp, &out, &secrets(&client));
                }
                Err(e) => {
                    save(&client);
                    eprintln!("ERROR: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: explore <login | csrf | req METHOD path [k=v …] [-H 'Name: value'] [--raw type body] [-o name]>");
            std::process::exit(2);
        }
    }
}
