//! A stateful stand-in for parfumo.com, for tests without network. It follows the behaviour
//! measured at jalon 0 bis (DISCOVERY §3.2, §3.3, §3.9, §4):
//! login sets `pf_auth`; `/api/*` answers 401 without a session and 403 without the
//! `csrf_key`; ShelfControl embeds the list as JSON; removing from "I have" moves to "I had".

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use serde_json::{Value, json};

pub const CSRF: &str = "0123456789abcdef0123456789abcdef";
pub const SAUVAGE: &str = "Dior/Sauvage_Eau_de_Toilette";
pub const SAUVAGE_P: u64 = 63363;

#[derive(Clone, Default)]
pub struct Row {
    wr: u64,
    p: u64,
    list: u64,
    qty: u32,
    bottle: String,
    ml: f64,
    paid: f64,
    cur: String,
    memo: String,
    custom: BTreeMap<String, String>,
    worn: u32,
}

#[derive(Default)]
pub struct Account {
    rows: Vec<Row>,
    /// Custom lists: id → (name, notes).
    custom_lists: BTreeMap<u64, (String, String)>,
    hidden: Vec<u64>,
    /// Wears: (l_id, p_id, `YYYY-MM-DDTHH:MM`, comment), newest first.
    wears: Vec<(u64, u64, String, String)>,
    /// Ratings: (p_id, scale) → value out of 100.
    votes: BTreeMap<(u64, String), u32>,
    /// Classification votes: (p_id, group id, label id).
    classi: Vec<(u64, u64, u64)>,
    /// Reviews by perfume: (id, title, text). Statements by perfume: (id, text).
    reviews: BTreeMap<u64, (u64, String, String)>,
    statements: BTreeMap<u64, (u64, String)>,
    next_id: u64,
}

#[derive(Clone, Default)]
pub struct Site {
    /// Parfumo accounts by username.
    pub accounts: Arc<Mutex<HashMap<String, Account>>>,
    /// Live `pf_auth` values → username.
    live: Arc<Mutex<HashMap<String, String>>>,
    pub logins: Arc<AtomicU32>,
    /// Every write received: (account, path, form body).
    pub writes: Arc<Mutex<Vec<(String, String, String)>>>,
    /// Every request: (path, cookie header).
    pub seen: Arc<Mutex<Vec<(String, String)>>>,
    /// Application interface (`/app/v1`): live session tokens → username.
    pub app_live: Arc<Mutex<HashMap<String, String>>>,
    pub app_logins: Arc<AtomicU32>,
    /// When set, the application interface refuses every login.
    pub app_login_refused: Arc<std::sync::atomic::AtomicBool>,
    /// Every call of the application interface made with a live token: (username, path).
    pub app_calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl Site {
    pub fn writes_to(&self, path: &str) -> usize {
        self.writes.lock().unwrap().iter().filter(|(_, p, _)| p == path).count()
    }

    pub fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }

    pub fn expire_sessions(&self) {
        self.live.lock().unwrap().clear();
    }
}

fn perfume(p: u64) -> (&'static str, &'static str, &'static str) {
    match p {
        SAUVAGE_P => ("Sauvage (Eau de Toilette)", "Dior", "Dior/Sauvage_Eau_de_Toilette"),
        111 => ("Bleu de Chanel", "Chanel", "Chanel/Bleu_de_Chanel"),
        _ => ("Terre d'Hermès", "Hermès", "Hermes/Terre_d_Hermes"),
    }
}

fn lists_json(a: &Account) -> Value {
    let count = |id: u64| a.rows.iter().filter(|r| r.list == id).count();
    let fixed = [
        (1, "have", "I have"),
        (2, "had", "I had"),
        (3, "want", "Wish List"),
        (4, "memo", "Watch List"),
        (5, "tested", "Tested"),
        (6, "filling", "Decants"),
        (7, "mini", "Miniatures"),
    ];
    let hidden = |id: u64| if a.hidden.contains(&id) { "Yes" } else { "No" };
    let mut out: Vec<Value> = fixed
        .iter()
        .map(|(id, slug, name)| json!({"id": id, "slug": slug, "name": name, "hidden": hidden(*id), "count": count(*id), "url": format!("https://mock/account/shelfcontrol/{slug}")}))
        .collect();
    for (id, (name, notes)) in &a.custom_lists {
        out.push(json!({"id": id, "slug": id.to_string(), "name": name, "notes": notes, "hidden": hidden(*id), "count": count(*id), "url": format!("https://mock/account/shelfcontrol/{id}")}));
    }
    Value::Array(out)
}

fn shelf_page(a: &Account, list: u64) -> String {
    let rows: Vec<Value> = a
        .rows
        .iter()
        .filter(|r| r.list == list)
        .map(|r| {
            let (title, _, slug) = perfume(r.p);
            json!({
                "id": r.wr, "p": r.p, "name": format!("{title} <span>x</span>"), "title": title,
                "url": format!("https://www.parfumo.com/Perfumes/{slug}"), "brand": r.p, "country": 74, "year": 2015, "gender": "maennlich",
                "conc": "Eau de Toilette", "ratings": [null, null, null, null, null], "community": 7.5, "votes": 100,
                "qty": r.qty, "bottle": r.bottle, "ml": r.ml, "paid": r.paid, "cur": r.cur, "memo": r.memo,
                "added": 1794464318, "added_f": "12/11/2026", "last": 0, "last_f": "", "worn": r.worn,
                "perfumers": [131], "accords": [17], "seasons": [9], "occasions": [5], "notes": [114, 2925],
                "custom": r.custom, "photos": [], "statement": 0, "st_text": "", "st_lang": "", "review": 0
            })
        })
        .collect();
    let brands: serde_json::Map<String, Value> = a.rows.iter().map(|r| (r.p.to_string(), json!([perfume(r.p).1, "https://mock/b"]))).collect();
    let data = json!({
        "rows": rows, "brands": brands, "countries": {"74": ["France", "<img>"]}, "genders": {"maennlich": "<i></i>"},
        "notes": {"114": "Ambrox", "2925": "Vetiver"}, "accords": {"17": "Fresh"}, "perfumers": {"131": ["François Demachy", "https://mock/p"]},
        "classes": {"9": "Spring", "5": "Daily"}, "groups": {"occasion": "Occasion", "season": "Season"}
    });
    let lists = lists_json(a).to_string().replace('&', "&amp;").replace('"', "&quot;");
    format!(
        "<html><head><title>ShelfControl</title></head><body><div class=\"psheet-wrap\" data-list=\"{list}\" data-lists=\"{lists}\" data-views=\"[]\">\
         <table><thead><tr><th data-col=\"country\">Country</th><th data-sort=\"text\" data-col=\"c178\">Gift</th></tr></thead></table></div>\
         <script type=\"application/json\" id=\"psheet-data\">{data}</script></body></html>"
    )
}

/// `2026-11-12T07:21` → `Thursday, 12/11/2026, 07:21 AM` (the weekday is not parsed).
fn site_when(iso: &str) -> String {
    let (date, time) = iso.split_once('T').unwrap();
    let d: Vec<&str> = date.split('-').collect();
    let (h, m) = time.split_once(':').unwrap();
    let h: u32 = h.parse().unwrap();
    let (h12, half) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("Thursday, {}/{}/{}, {h12:02}:{m} {half}", d[2], d[1], d[0])
}

fn wear_rows(a: &Account, month: Option<&str>) -> String {
    a.wears
        .iter()
        .filter(|w| month.is_none_or(|m| w.2.starts_with(m)))
        .map(|(l, p, when, comment)| {
            let (title, brand, slug) = perfume(*p);
            let note = if comment.is_empty() { String::new() } else { format!(" <i class=\"fa fa-sticky-note lightgrey\" title=\"{comment}\"></i>") };
            format!(
                "<div class=\"col\" id=\"tablerow{l}\"><div class=\"image\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\"><img alt=\"{title} by {brand}\"></a></div>\
                 <div class=\"name\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\">{title}</a><br />\
                 <div class=\"action_tracker_details pointer\" data-l_id=\"{l}\">{}{note}</div></div></div>",
                site_when(when)
            )
        })
        .collect()
}

fn redirect(to: &str) -> Response {
    Response::builder().status(StatusCode::FOUND).header(header::LOCATION, to).body(Body::empty()).unwrap()
}

async fn handler(State(site): State<Site>, method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let path = uri.path().to_string();
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    site.seen.lock().unwrap().push((path.clone(), cookie.clone()));
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let f = |k: &str| form.get(k).cloned().unwrap_or_default();
    let n = |k: &str| form.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);

    if path == "/api/alive" {
        return format!("{{\"csrf_key\":\"{CSRF}\"}}").into_response();
    }
    if let Some(call) = path.strip_prefix("/app/v1") {
        let json = |status: StatusCode, text: String| (status, [(header::CONTENT_TYPE, "application/json")], text).into_response();
        if call == "/auth/login" {
            let count = site.app_logins.fetch_add(1, Ordering::SeqCst) + 1;
            let user = f("email");
            let known = site.accounts.lock().unwrap().contains_key(&user);
            if site.app_login_refused.load(Ordering::SeqCst) || !known || f("password") != format!("pw-{user}") || f("device_id").is_empty() {
                return json(StatusCode::UNAUTHORIZED, "{\"status\":\"error\",\"message\":\"Invalid credentials\"}".into());
            }
            let token = format!("app-{user}-{count}");
            site.app_live.lock().unwrap().insert(token.clone(), user.clone());
            return json(StatusCode::OK, format!("{{\"status\":\"success\",\"auth_token\":\"{token}\",\"nick_name\":\"{user}\"}}"));
        }
        let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
        let Some(user) = site.app_live.lock().unwrap().get(bearer).cloned() else {
            return json(StatusCode::UNAUTHORIZED, "{\"status\":\"error\",\"message\":\"Authorization header required\"}".into());
        };
        site.app_calls.lock().unwrap().push((user.clone(), call.to_string()));
        return match call {
            "/auth/me" => json(StatusCode::OK, format!("{{\"status\":\"success\",\"user\":{{\"nick_name\":\"{user}\"}}}}")),
            // One row of the filtered listing, whatever is asked.
            "/perfumes" => json(
                StatusCode::OK,
                "{\"matches\":1,\"rows\":[{\"p_id\":\"63363\",\"p_name\":\"Sauvage (Eau de Toilette)\",\"p_name_url\":\"Sauvage_Eau_de_Toilette\",\
                 \"b_name\":\"Dior\",\"b_name_url\":\"Dior\",\"p_release_year\":\"2015\",\"p_gender\":\"maennlich\",\"p_total_votings\":\"100\",\"p_avg_voting\":\"80\"}]}"
                    .into(),
            ),
            _ => json(StatusCode::NOT_FOUND, "{\"error\":\"Not found\"}".into()),
        };
    }
    if path == "/api/account/login" {
        let count = site.logins.fetch_add(1, Ordering::SeqCst) + 1;
        let user = f("username");
        if f("password") != format!("pw-{user}") || !site.accounts.lock().unwrap().contains_key(&user) {
            return redirect("/account/login_error");
        }
        let token = format!("{:0>60}{count:04}", user.len());
        site.live.lock().unwrap().insert(token.clone(), user);
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/")
            .header(header::SET_COOKIE, format!("pf_auth={token}; Max-Age=315360000; Path=/; Secure; HttpOnly"))
            .header(header::SET_COOKIE, format!("PHPSESSID=sess{count}; Path=/"))
            .body(Body::empty())
            .unwrap();
    }
    if path == "/Perfumes/Dior/Sauvage_Eau_de_Toilette" {
        let fixture = format!("{}/../parfumo-client/tests/fixtures/perfume_sauvage.html", env!("CARGO_MANIFEST_DIR"));
        return Html(std::fs::read_to_string(fixture).unwrap()).into_response();
    }

    // Everything below belongs to an account.
    let user = cookie.split(';').filter_map(|c| c.trim().strip_prefix("pf_auth=")).find_map(|token| site.live.lock().unwrap().get(token).cloned());
    let Some(user) = user else {
        return if path.starts_with("/api/") {
            (StatusCode::UNAUTHORIZED, "{\"error\":\"Authentication required\"}").into_response()
        } else if path.starts_with("/action/") && method == Method::POST {
            "".into_response()
        } else {
            redirect("/action/dologin")
        };
    };
    let mut accounts = site.accounts.lock().unwrap();
    let a = accounts.get_mut(&user).unwrap();

    if method == Method::GET {
        return match path.as_str() {
            "/account/notifications" => {
                Html(format!("<html><head><title>Notifications</title></head><body><span class=\"nick_name\">{user} <i></i></span>ok</body></html>")).into_response()
            }
            p if p.starts_with(&format!("/Users/{user}/")) => member_page(a, &user, &p[format!("/Users/{user}/").len()..]),
            "/assistant/tracker" => Html(format!(
                "<html><head><title>Tracker</title></head><body><a href=\"https://mock/assistant/tracker_calendar\">Calendar</a><div class=\"pgrid\">{}</div></body></html>",
                wear_rows(a, None)
            ))
            .into_response(),
            "/action/wardrobe/generate_wardrobe_csv.php" => {
                let list: u64 = uri.query().and_then(|q| q.strip_prefix("list_id=")).and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut csv = "\u{feff}\"Perfume\",\"Brand\",\"Year\",\"Gender\",\"Quantity\",\"Bottle Size\",\"Paid\",\"Date of Entry\",\"Notes\",\"Notebook\"\n".to_string();
                for r in a.rows.iter().filter(|r| r.list == list) {
                    csv.push_str(&format!("\"{}\",\"{}\",\"2015\",\"Men\",\"{}\",\"{}\",\"\",\"2026-11-12\",\"\",\"\"\n", perfume(r.p).0, perfume(r.p).1, r.qty, r.bottle));
                }
                csv.into_response()
            }
            p if p == "/account/shelfcontrol" || p.starts_with("/account/shelfcontrol/") => {
                let slug = p.trim_start_matches("/account/shelfcontrol").trim_start_matches('/');
                let id = match slug {
                    "" | "have" => 1,
                    "had" => 2,
                    "want" => 3,
                    "memo" => 4,
                    "tested" => 5,
                    "filling" => 6,
                    "mini" => 7,
                    other => other.parse().ok().filter(|id| a.custom_lists.contains_key(id)).unwrap_or(1),
                };
                Html(shelf_page(a, id)).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        };
    }

    // POST: every internal call needs the session csrf_key.
    if f("csrf_key") != CSRF {
        return if path.starts_with("/api/") {
            (StatusCode::FORBIDDEN, "{\"error\":\"Invalid CSRF token\"}").into_response()
        } else {
            "".into_response()
        };
    }
    if path == "/api/assistant/calendar_month" {
        let month = format!("{:04}-{:02}", n("y"), n("m"));
        return format!(
            "<div class=\"grey-box tcal\" data-y=\"{}\" data-m=\"{}\"></div><div class=\"tcal-panels\">{}</div>",
            n("y"),
            n("m"),
            wear_rows(a, Some(&month))
        )
        .into_response();
    }
    if let Some(read) = feedback_read(a, &path, n("perfume_id").max(n("p")), n("review_id")) {
        return read;
    }
    site.writes.lock().unwrap().push((user.clone(), path.clone(), String::from_utf8_lossy(&body).replace(&format!("&csrf_key={CSRF}"), "")));
    a.next_id += 1;
    let fresh = 1000 * (user.len() as u64) + a.next_id;
    match path.as_str() {
        "/api/wardrobe/wardrobe_add_remove" => {
            let (p, list) = (n("p"), n("t"));
            if f("action") == "true" {
                if p == 999 {
                    return "0".into_response();
                }
                if a.rows.iter().any(|r| r.p == p && r.list == list) {
                    return "".into_response();
                }
                a.rows.push(Row { wr: fresh, p, list, qty: 1, ..Default::default() });
                fresh.to_string().into_response()
            } else {
                if let Some(i) = a.rows.iter().position(|r| r.p == p && r.list == list) {
                    let mut row = a.rows.remove(i);
                    if list == 1 && !a.rows.iter().any(|r| r.p == p && r.list == 2) {
                        row.list = 2;
                        a.rows.push(row);
                    }
                }
                "".into_response()
            }
        }
        "/api/wardrobe/wardrobe_copy_move" => {
            let (wr, to) = (n("wr_id"), n("new_list_id"));
            let Some(i) = a.rows.iter().position(|r| r.wr == wr && r.list == n("list_id")) else {
                return "{\"error_title\":\"Move not possible\",\"error_message\":\"Unknown entry\"}".into_response();
            };
            if f("copy_move") == "copy" {
                let mut copy = a.rows[i].clone();
                (copy.wr, copy.list) = (fresh, to);
                a.rows.push(copy);
            } else {
                a.rows[i].list = to;
            }
            "1".into_response()
        }
        "/api/wardrobe/update_details" => {
            // Like the real site: answers "ok" even for an entry of another account.
            if let Some(r) = a.rows.iter_mut().find(|r| r.wr == n("id")) {
                r.qty = n("quantity") as u32;
                r.bottle = f("bottle_size");
                r.paid = f("paid").parse().unwrap_or(0.0);
                r.cur = f("currency");
                "{\"status\":\"ok\",\"date_added\":\"12\\/11\\/2026\"}".into_response()
            } else {
                "{\"status\":\"ok\",\"date_added\":\"\"}".into_response()
            }
        }
        "/action/shelfcontrol/columns.php" => {
            if let Some(r) = a.rows.iter_mut().find(|r| r.wr == n("row")) {
                match f("op").as_str() {
                    "ml" => r.ml = f("value").parse().unwrap_or(0.0),
                    _ => {
                        r.custom.insert(f("id"), f("value"));
                    }
                }
            }
            "{\"ok\":true}".into_response()
        }
        "/api/memo/create_memo" => {
            if let Some(r) = a.rows.iter_mut().find(|r| r.wr == n("id")) {
                r.memo = f("text");
            }
            json!({"status": "ok", "text": f("text"), "date": "12/11/2026, 07:19 AM"}).to_string().into_response()
        }
        "/api/memo/delete_memo" => {
            if let Some(r) = a.rows.iter_mut().find(|r| r.wr == n("id")) {
                r.memo.clear();
            }
            "".into_response()
        }
        "/api/wardrobe/add_list" => {
            let id = 900_000 + fresh;
            a.custom_lists.insert(id, ("New Collection".into(), String::new()));
            json!({"status": "ok", "id": id}).to_string().into_response()
        }
        "/api/wardrobe/set_wardrobe_name_notes" => match a.custom_lists.get_mut(&n("id")) {
            Some(l) => {
                *l = (f("name"), f("notes"));
                json!({"name": f("name"), "notes": f("notes")}).to_string().into_response()
            }
            None => "".into_response(),
        },
        "/api/wardrobe/wardrobe_lock_toggle" => {
            let id = n("id");
            if let Some(i) = a.hidden.iter().position(|h| *h == id) {
                a.hidden.remove(i);
                "No".into_response()
            } else {
                a.hidden.push(id);
                "Yes".into_response()
            }
        }
        "/api/wardrobe/del_cust_wr" => {
            let id = n("id");
            a.custom_lists.remove(&id);
            a.rows.retain(|r| r.list != id);
            "".into_response()
        }
        "/api/assistant/i_wear" => {
            if f("p") != "clear" {
                a.wears.insert(0, (fresh, n("p"), "2026-11-12T07:21".into(), String::new()));
            }
            "<div class=\"text-center\">What are you wearing?</div>".into_response()
        }
        "/api/account/update_iwear_date" => match a.wears.iter_mut().find(|w| w.0 == n("id")) {
            Some(w) => {
                w.2 = f("new_date");
                "1".into_response()
            }
            None => "".into_response(),
        },
        "/api/account/update_iwear_comment" => match a.wears.iter_mut().find(|w| w.0 == n("id")) {
            Some(w) => {
                w.3 = f("comment");
                "1".into_response()
            }
            None => "".into_response(),
        },
        "/api/account/delete_iwear" => {
            a.wears.retain(|w| w.0 != n("id"));
            "".into_response()
        }
        "/api/perfume/add_vote" => {
            a.votes.insert((n("p_id"), f("type")), n("v") as u32);
            "{\"status\":\"ok\"}".into_response()
        }
        "/api/perfume/remove_vote" => {
            a.votes.remove(&(n("p_id"), f("type")));
            "{\"status\":\"ok\"}".into_response()
        }
        "/api/perfume/toggle_classification_vote" => {
            let vote = (n("p"), n("c"), n("ct"));
            a.classi.retain(|v| *v != vote);
            if f("action") == "add" {
                a.classi.push(vote);
                "1".into_response()
            } else {
                "0".into_response()
            }
        }
        "/api/review/create_review" => {
            if f("text").contains("REFUSE") {
                return "{\"status\":0,\"msg\":\"Your review is too short.\"}".into_response();
            }
            let id = a.reviews.get(&n("id")).map(|r| r.0).unwrap_or(fresh);
            a.reviews.insert(n("id"), (id, f("title"), f("text")));
            json!({"status": 1, "url": format!("https://mock/reviews/{id}")}).to_string().into_response()
        }
        "/api/statement/create_statement" => {
            if f("text").contains("REFUSE") {
                return "{\"status\":0,\"msg\":\"Please write a real statement.\"}".into_response();
            }
            let id = a.statements.get(&n("id")).map(|r| r.0).unwrap_or(fresh);
            a.statements.insert(n("id"), (id, f("text")));
            json!({"status": 1, "h": CSRF, "url": format!("https://mock/statements/{id}")}).to_string().into_response()
        }
        "/api/review/delete_review" => {
            if a.reviews.get(&n("p")).is_some_and(|r| r.0 == n("id")) && f("h") == delete_token(n("id")) {
                a.reviews.remove(&n("p"));
            }
            "{\"status\":1}".into_response()
        }
        "/api/statement/delete_statement" => {
            if a.statements.get(&n("p")).is_some_and(|r| r.0 == n("id")) && f("h") == delete_token(n("id")) {
                a.statements.remove(&n("p"));
            }
            "{\"status\":1}".into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---- jalon 3: ratings, classification, reviews and statements ------------------------------

const SCALES: [&str; 5] = ["scent", "durability", "sillage", "bottle", "pricing"];
/// Classification labels of the stand-in: (group id, group, label id, label).
const LABELS: [(u64, &str, u64, &str); 7] = [
    (4, "Type", 17, "Fresh"),
    (4, "Type", 19, "Woody"),
    (4, "Type", 23, "Spicy"),
    (3, "Season", 9, "Spring"),
    (3, "Season", 10, "Summer"),
    (2, "Occasion", 5, "Daily"),
    (2, "Occasion", 7, "Night Out"),
];

/// Token carried by the delete button of a review or statement form.
pub fn delete_token(id: u64) -> String {
    format!("{id:032x}")
}

fn stars(p: u64, scale: &str, tenths: u32) -> String {
    format!(
        "<div class=\"pstars star stars_{scale} stars_{scale}_{p}\" data-pstars-step=\"0.25\" data-pstars-value=\"{}\"></div>",
        f64::from(tenths) / 20.0
    )
}

/// Read-only internal calls of jalon 3 (forms and own texts). `None` for any other route.
fn feedback_read(a: &Account, path: &str, p: u64, review_id: u64) -> Option<Response> {
    let form = |kind: &str, existing: Option<u64>, show: bool| {
        let button = match existing.filter(|_| show) {
            Some(id) => format!(
                "<button class=\"action_delete_{kind} pbtn pbtn-pink\" type=\"button\" data-id=\"{id}\" data-p=\"{p}\" data-h=\"{}\">Delete</button>",
                delete_token(id)
            ),
            None => String::new(),
        };
        format!(
            "<h2>Form</h2><form><input type=\"hidden\" name=\"p\" id=\"form_perfume_id\" value=\"{p}\"><div class=\"pbtn-row\">{button}</div></form>"
        )
    };
    Some(match path {
        "/action/perfume/perfume_rate_panel.php" => SCALES
            .iter()
            .map(|scale| {
                let vote = a.votes.get(&(p, scale.to_string()));
                format!(
                    "<div class=\"vote_element\"><a class=\"pointer vote_remove_{p}_{scale}{}\"></a>{}</div>",
                    if vote.is_some() { "" } else { " hide" },
                    stars(p, scale, vote.copied().unwrap_or(0))
                )
            })
            .collect::<String>()
            .into_response(),
        "/action/perfume/perfume_classify_panel.php" => {
            let mut html = String::new();
            for (c, group) in [(4, "Type"), (3, "Season"), (2, "Occasion")] {
                html.push_str(&format!("<div class=\"classification_group\"><div class=\"flex vm bold\"><svg></svg>{group}</div><div>"));
                for (_, _, ct, label) in LABELS.iter().filter(|l| l.0 == c) {
                    let class = if a.classi.contains(&(p, c, *ct)) { "pbtn-green" } else { "pbtn-lightgrey2" };
                    html.push_str(&format!(
                        "<div class=\"action_add_classi pbtn pbtn-tiny {class}\" data-p=\"{p}\" data-c=\"{c}\" data-ct=\"{ct}\">{label}</div>"
                    ));
                }
                html.push_str("</div></div>");
            }
            html.into_response()
        }
        // Stand-in choice: the review form only shows its delete button when it is asked
        // for with the review id, the statement form always does. Both paths are covered.
        "/action/perfume/perfume_review_panel.php" => {
            let existing = a.reviews.get(&p).map(|r| r.0);
            form("review", existing, existing == Some(review_id)).into_response()
        }
        "/action/perfume/perfume_statement_panel.php" => form("statement", a.statements.get(&p).map(|s| s.0), true).into_response(),
        "/api/review/get_user_review" => {
            let (title, text) = a.reviews.get(&p).map(|r| (r.1.clone(), r.2.clone())).unwrap_or_default();
            json!({"review_title": title, "review_text": text}).to_string().into_response()
        }
        "/api/statement/get_user_statement" => {
            json!({"statement_text": a.statements.get(&p).map(|s| s.1.clone()).unwrap_or_default()}).to_string().into_response()
        }
        _ => return None,
    })
}

/// The member's own pages: `Ratings/{scale}`, `reviews`, `statements`.
fn member_page(a: &Account, user: &str, rest: &str) -> Response {
    let empty = "<div class=\"text-center\"><img class=\"nothing_info_img\"><div>Nothing yet.</div></div>";
    let body = if let Some(scale) = rest.strip_prefix("Ratings/") {
        let items: String = a
            .votes
            .iter()
            .filter(|((_, s), _)| s == scale)
            .map(|((p, _), tenths)| {
                let (title, brand, slug) = perfume(*p);
                format!(
                    "<div class=\"user-ratings-item\" id=\"tablerow{p}\"><div class=\"image\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\"><img alt=\"{title} by {brand}\"></a></div>\
                     <div class=\"content\"><div class=\"name\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\">{title}</a><div class=\"time-ago\">Just now</div>{}</div></div></div>",
                    stars(*p, scale, *tenths)
                )
            })
            .collect();
        format!(
            "<nav><a href=\"https://mock/Users/{user}/Ratings/scent\">Scent</a></nav>{}",
            if items.is_empty() { empty.to_string() } else { items }
        )
    } else if rest == "reviews" {
        let items: String = a
            .reviews
            .iter()
            .map(|(p, (id, title, text))| {
                let (name, brand, slug) = perfume(*p);
                format!(
                    "<div class=\"review-box-user-reviews-page grey-box review_box_full_{id}\"><div class=\"header_left\"><span class=\"text-xs lightblue2\">Just now</span></div>\
                     <div class=\"header_right\"><a href=\"https://www.parfumo.com/Perfumes/{slug}/reviews/{id}\">open</a></div>\
                     <div class=\"image\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\"><img></a><div><a href=\"https://www.parfumo.com/Perfumes/{slug}\" class=\"bold\">{name}</a> \
                     <a href=\"https://www.parfumo.com/Perfumes/{brand}\" class=\"text-sm lightblue nobold\">- {brand}</a></div></div>\
                     <div id=\"r_title_{id}\">{title}</div><div id=\"r_text_{id}\">{text}</div></div>"
                )
            })
            .collect();
        format!("<h1>{user}</h1>{}", if items.is_empty() { empty.to_string() } else { items })
    } else if rest == "statements" {
        let items: String = a
            .statements
            .iter()
            .map(|(p, (id, text))| {
                let (name, brand, slug) = perfume(*p);
                format!(
                    "<div class=\"statement mb-1\" id=\"tablerow{id}\"><a href=\"https://www.parfumo.com/Perfumes/{slug}\" class=\"title bold\">{name}</a>\
                     <a href=\"https://www.parfumo.com/Perfumes/{brand}\" class=\"text-sm lightblue\">{brand}</a><div id=\"s_text_content_{id}\">{text}</div></div>"
                )
            })
            .collect();
        format!("<h1>{user}</h1>{}", if items.is_empty() { empty.to_string() } else { items })
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Html(format!("<html><head><title>{user}</title></head><body><div class=\"main\">{body}</div></body></html>")).into_response()
}

pub async fn spawn_site() -> (String, Site) {
    let site = Site::default();
    for user in ["alice", "bob"] {
        site.accounts.lock().unwrap().insert(user.into(), Account::default());
    }
    let app = Router::new().fallback(handler).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), site)
}
