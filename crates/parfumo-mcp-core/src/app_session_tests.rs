//! The session of the application interface (`/app/v1`) of a linked account: obtained with
//! the link, kept across a restart, held in one place per user, renewed once when the site
//! ends it, never insisted on.
//!
//! The stand-in site records every request it receives under `/app/v1` with the token it
//! carried, whatever it answers: a request without a token, or with a dead one, is seen.

use std::{sync::atomic::Ordering, time::Duration};

use parfumo_client::{ClientError, SearchQuery};
use serde_json::{Value, json};

use crate::{
    caller::Caller,
    config::Config,
    crypto::MasterKey,
    db::{Db, User},
    sessions::{Credentials, LocalAccount},
    state::AppState,
    testing::{
        TestApp, error_code, link_account, linked, ok,
        site::{Site, spawn_site},
        test_config,
    },
    tools::{LinkHow, ParfumoTools},
};

/// What the stand-in answers to a search: one row through the application interface, the
/// twenty of the recorded page through the web site.
const FROM_THE_INTERFACE: usize = 1;
const FROM_THE_WEB: usize = 20;

/// The pages of the web site asked since the last call, and every request received under
/// `/app/v1` with the token it carried (none for a login).
fn asked(site: &Site) -> (Vec<String>, Vec<(String, String)>) {
    let web = site.seen.lock().unwrap().drain(..).map(|(path, _)| path).filter(|p| !p.starts_with("/app/v1")).collect();
    (web, site.app_seen.lock().unwrap().drain(..).collect())
}

fn call(path: &str, token: &str) -> (String, String) {
    (path.to_string(), token.to_string())
}

fn login() -> (String, String) {
    call("/auth/login", "")
}

fn logins(site: &Site) -> (u32, u32) {
    (site.logins.load(Ordering::SeqCst), site.app_logins.load(Ordering::SeqCst))
}

fn rows(result: &Value) -> usize {
    ok(result)["items"].as_array().unwrap_or_else(|| panic!("{result}")).len()
}

/// Whether the web site was asked, and for its search only. (The search tool reads on
/// until it has enough results: the stand-in is asked for more pages than one.)
fn searched_the_web(web: &[String]) -> bool {
    !web.is_empty() && web.iter().all(|path| path == "/s_perfumes_x.php")
}

/// One process of the server, on a database and a key that outlive it: starting another
/// one on them is a restart. It calls the common paths of the tools directly, so that
/// calls of one user can be made at the same time.
struct Process {
    state: AppState,
    tools: ParfumoTools,
    user: User,
}

struct World {
    base: String,
    site: Site,
    db: Db,
    key: String,
    user: i64,
}

impl World {
    /// A stand-in site and a database with one user, whose requests are `interval_ms` apart.
    async fn new() -> Self {
        let (base, site) = spawn_site().await;
        let db = Db::open_in_memory().unwrap();
        let user = db.add_user("someone", "hash").unwrap();
        Self { base, site, db, key: MasterKey::generate_base64(), user }
    }

    fn start(&self) -> Process {
        self.start_with(test_config(&self.base))
    }

    fn start_with(&self, cfg: Config) -> Process {
        let state = AppState::new(cfg, self.db.clone(), MasterKey::from_base64(&self.key).unwrap());
        let user = User { id: self.user, name: "someone".into() };
        Process { tools: ParfumoTools::local(state.clone(), user.clone(), LinkHow::Environment), state, user }
    }

    fn link(&self) -> crate::db::LinkRow {
        self.db.link_get(self.user).unwrap().unwrap()
    }
}

impl Process {
    /// A search through the common path of the database tools: the number of perfumes found.
    async fn search(&self) -> usize {
        let result = self
            .tools
            .run(&Caller(self.user.clone()), "parfumo_search_perfumes", "sauvage", |client| async move {
                let query = SearchQuery { text: Some("sauvage".into()), ..SearchQuery::default() };
                let found = client.search_perfumes(&query, 1).await.map_err(|e| self.tools.client_err(e))?;
                Ok((String::new(), json!({ "rows": found.items.len() })))
            })
            .await
            .unwrap();
        let result = serde_json::to_value(result).unwrap();
        result["structuredContent"]["rows"].as_u64().unwrap_or_else(|| panic!("{result}")) as usize
    }

    /// The member's nickname through the common path of the account tools.
    async fn own_nick(&self) -> String {
        let result = self
            .tools
            .run_account(&Caller(self.user.clone()), "parfumo_list_collections", "", |clients| async move {
                let nick = clients.account.own_nick().await.map_err(|e| self.tools.client_err(e))?;
                Ok((String::new(), json!({ "nick": nick })))
            })
            .await
            .unwrap();
        let result = serde_json::to_value(result).unwrap();
        result["structuredContent"]["nick"].as_str().unwrap_or_else(|| panic!("{result}")).to_string()
    }

    async fn link_to(&self, name: &str) {
        link_account(&self.state, self.user.id, name).await;
    }

    /// Whether the user's clients hold a session of the application interface.
    fn holds_session(&self) -> bool {
        self.state.sessions.public_client_for(self.user.id).unwrap().has_app_session()
    }
}

#[tokio::test]
async fn a_linked_account_gets_keeps_and_renews_its_session_of_the_application_interface() {
    let w = World::new().await;
    let (site, user) = (&w.site, w.user);
    let p = w.start();

    // Linking logs in to the web site and to the application interface, once each.
    p.link_to("alice").await;
    assert_eq!(logins(site), (1, 1));
    let client = p.state.sessions.account_client(user).await.unwrap();
    assert!(client.has_app_session());
    assert_eq!(client.app_nick().await.unwrap(), "alice");

    // A restart finds the token with the cookies: no new login of either kind.
    let restarted = w.start();
    let again = restarted.state.sessions.account_client(user).await.unwrap();
    assert_eq!(again.app_nick().await.unwrap(), "alice");
    assert_eq!(logins(site), (1, 1));
    // Neither the token nor the password is stored in clear.
    let stored = w.link();
    for blob in [stored.cookies.unwrap(), stored.secret.unwrap()] {
        let text = String::from_utf8_lossy(&blob).into_owned();
        assert!(!text.contains("app-alice") && !text.contains("pw-alice"), "a secret is stored in clear");
    }

    // The site ends the session: the refused token is forgotten, and the next check of the
    // account logs in to the application interface again, once, without a web login.
    site.app_live.lock().unwrap().clear();
    assert_eq!(client.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert!(!client.has_app_session());
    let renewed = p.state.sessions.renew(user).await.unwrap();
    assert_eq!(renewed.app_nick().await.unwrap(), "alice");
    assert_eq!(logins(site), (1, 2));

    // The renewed session is lost at once: no third login, call after call. The account
    // stays linked, on the web session.
    site.app_live.lock().unwrap().clear();
    assert_eq!(renewed.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    for _ in 0..3 {
        assert!(!p.state.sessions.renew(user).await.unwrap().has_app_session());
    }
    assert_eq!(logins(site), (1, 2), "one renewal, then a pause: not a login at every call");
    // After the pause, one new attempt.
    p.state.sessions.end_app_backoff(user);
    assert!(p.state.sessions.renew(user).await.unwrap().has_app_session());
    assert_eq!(logins(site), (1, 3));

    // The interface refuses the login itself: one attempt, and no other, pause or not.
    site.app_live.lock().unwrap().clear();
    assert_eq!(renewed.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    site.app_login_refused.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        p.state.sessions.end_app_backoff(user);
        assert!(!p.state.sessions.renew(user).await.unwrap().has_app_session());
    }
    assert_eq!(logins(site), (1, 4), "refused credentials are not sent again");
}

#[tokio::test]
async fn a_database_tool_reads_through_the_session_of_the_linked_account_and_the_web_site_otherwise() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (alice, alice_key) = linked(&app, "alice").await;
    let (_, bob_key) = app.add_user("bob");
    asked(&site);
    let search = json!({"query": "sauvage"});

    // Linked: the listing of the application interface, asked as alice, and no web page.
    let found = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(rows(&found), FROM_THE_INTERFACE, "{found}");
    assert_eq!(ok(&found)["items"][0]["id"], "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(ok(&found)["items"][0]["gender"], "men");
    let (web, interface) = asked(&site);
    assert_eq!(interface, [call("/perfumes", "app-alice-1")]);
    assert_eq!(*site.app_calls.lock().unwrap(), [("alice".to_string(), "/perfumes".to_string())]);
    assert!(web.is_empty(), "{web:?}");

    // Not linked: nothing at all reaches the interface, with a token or without; the
    // search of the web site answers.
    let from_the_web = app.call_tool(&bob_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(rows(&from_the_web), FROM_THE_WEB);
    let (web, interface) = asked(&site);
    assert_eq!(interface, [], "a user without an account asked the interface");
    assert!(searched_the_web(&web), "{web:?}");

    // The site ends alice's session: the same call is answered by the web site at once,
    // exactly as for a user without an account. The dead token left once, and no login was
    // made in the middle of the call.
    site.app_live.lock().unwrap().clear();
    let answer = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(ok(&answer)["items"], ok(&from_the_web)["items"]);
    let (web, interface) = asked(&site);
    assert_eq!(interface, [call("/perfumes", "app-alice-1")]);
    assert!(searched_the_web(&web), "{web:?}");

    // The token is gone for the account too: the next call logs in again, once, before
    // reading, and the interface is used again.
    let found = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(rows(&found), FROM_THE_INTERFACE);
    assert_eq!(asked(&site), (vec![], vec![login(), call("/perfumes", "app-alice-2")]));
    assert!(app.state.sessions.account_client(alice).await.unwrap().has_app_session());

    // No token in what the tools answered, nor in the audit log.
    let audit = format!("{:?}", app.state.db.audit_list(None, 50).unwrap());
    for told in [found.to_string(), answer.to_string(), from_the_web.to_string(), audit] {
        assert!(!told.contains("app-alice") && !told.contains("pw-alice"), "{told}");
    }
}

#[tokio::test]
async fn the_page_reader_sends_nothing_to_the_application_interface() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, alice_key) = linked(&app, "alice").await;
    let (_, bob_key) = app.add_user("bob");
    asked(&site);
    // With a linked account or without, by a path or by a whole address.
    for key in [&alice_key, &bob_key] {
        for url in ["/app/v1/auth/me".to_string(), "/APP/v1/perfumes?filter=x".to_string(), format!("{base}/app/v1/auth/me")] {
            let refused = app.call_tool(key, "parfumo_fetch_page", json!({ "url": url })).await;
            assert_eq!(error_code(&refused), "invalid_input", "{url}");
        }
    }
    assert_eq!(asked(&site).1, [], "a request without a token reached the interface: two of them block the address");
}

#[tokio::test]
async fn calls_made_together_send_a_refused_token_once_and_log_in_once() {
    let w = World::new().await;
    let site = &w.site;
    // Requests one slot apart, as in production: calls made together queue.
    let p = w.start_with(Config { user_min_interval: Duration::from_millis(20), ..test_config(&w.base) });
    p.link_to("alice").await;
    asked(site);

    // The site ends the session, and the assistant sends three reads at once.
    site.app_live.lock().unwrap().clear();
    let found = tokio::join!(p.search(), p.search(), p.search());
    assert_eq!(found, (FROM_THE_WEB, FROM_THE_WEB, FROM_THE_WEB), "every read is answered by the web site");
    let (web, interface) = asked(site);
    assert_eq!(interface, [call("/perfumes", "app-alice-1")], "the dead token left once, and no login in the middle of the calls");
    assert_eq!(web, ["/s_perfumes_x.php"; 3]);

    // No session is held: four reads at once log in once, and all use what it obtained.
    let found = tokio::join!(p.search(), p.search(), p.search(), p.search());
    assert_eq!(found, (FROM_THE_INTERFACE, FROM_THE_INTERFACE, FROM_THE_INTERFACE, FROM_THE_INTERFACE));
    let (_, interface) = asked(site);
    assert_eq!(interface[0], login());
    assert_eq!(interface[1..], vec![call("/perfumes", "app-alice-2"); 4], "one login for the four calls");

    // The same when the interface refuses the login: one attempt, not four.
    site.app_live.lock().unwrap().clear();
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_WEB);
    asked(site);
    site.app_login_refused.store(true, Ordering::SeqCst);
    let found = tokio::join!(p.search(), p.search(), p.search(), p.search());
    assert_eq!(found, (FROM_THE_WEB, FROM_THE_WEB, FROM_THE_WEB, FROM_THE_WEB));
    assert_eq!(asked(site).1, [login()], "a refused login was repeated by the calls that waited for it");
}

#[tokio::test]
async fn a_call_that_starts_while_another_falls_back_does_not_bring_the_refused_token_back() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    asked(site);

    // The site ends the session, and its web pages are slow: the first call meets the
    // refusal, then waits for the web site.
    site.app_live.lock().unwrap().clear();
    site.web_delay_ms.store(300, Ordering::SeqCst);
    let second = async {
        while p.holds_session() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Started in the middle of the first one: it logs in again and reads at once.
        site.web_delay_ms.store(0, Ordering::SeqCst);
        p.search().await
    };
    assert_eq!(tokio::join!(p.search(), second), (FROM_THE_WEB, FROM_THE_INTERFACE));
    let (_, interface) = asked(site);
    assert_eq!(interface, [call("/perfumes", "app-alice-1"), login(), call("/perfumes", "app-alice-2")], "the refused token left again");

    // The end of the first call did not take the new session away: not in memory…
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [call("/perfumes", "app-alice-2")]);
    // … nor in the database.
    assert_eq!(w.start().search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [call("/perfumes", "app-alice-2")], "what is stored is the session obtained last");
}

#[tokio::test]
async fn a_session_refused_again_right_after_its_renewal_is_not_asked_for_at_every_call() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    asked(site);

    // The interface accepts every login and refuses every token.
    site.app_tokens_refused.store(true, Ordering::SeqCst);
    for _ in 0..5 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(
        asked(site).1,
        [call("/perfumes", "app-alice-1"), login(), call("/perfumes", "app-alice-2")],
        "one renewal, one more refusal, then the web site alone: not a login and a refusal at every call"
    );
    // The account tools follow the same rule.
    for _ in 0..3 {
        assert!(!p.state.sessions.renew(w.user).await.unwrap().has_app_session());
    }
    assert_eq!(asked(site).1, []);

    // Once the pause is over, one renewal again, and no more.
    p.state.sessions.end_app_backoff(w.user);
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-3")]);
}

#[tokio::test]
async fn credentials_the_interface_refused_are_never_sent_to_it_again_until_the_account_is_linked_again() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();

    // Refused when the account is linked: the web site accepted the credentials, so the
    // account is linked, and the interface was asked once.
    site.app_login_refused.store(true, Ordering::SeqCst);
    p.link_to("alice").await;
    assert_eq!(logins(site), (1, 1));
    assert!(w.link().app_refused && w.link().status == "linked");
    // The first reads do not repeat the login.
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    // It is not a pause: it does not end with time.
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_WEB);
    assert!(!p.state.sessions.renew(w.user).await.unwrap().has_app_session());
    // Nor with a restart.
    let restarted = w.start();
    assert_eq!(restarted.search().await, FROM_THE_WEB);
    assert_eq!(restarted.own_nick().await, "alice");
    assert_eq!(logins(site), (1, 1), "the refused credentials were sent to the interface again");
    assert_eq!(asked(site).1, [login()], "nothing but the one login ever reached the interface");

    // Linking the account again clears the refusal.
    site.app_login_refused.store(false, Ordering::SeqCst);
    restarted.link_to("alice").await;
    assert_eq!(logins(site), (2, 2));
    assert!(!w.link().app_refused);
    assert_eq!(restarted.search().await, FROM_THE_INTERFACE);
}

#[tokio::test]
async fn an_outage_of_the_login_to_the_interface_is_followed_by_a_pause_not_taken_for_a_refusal() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();

    site.app_login_unavailable.store(true, Ordering::SeqCst);
    p.link_to("alice").await;
    assert_eq!(logins(site), (1, 1));
    assert!(!w.link().app_refused, "an outage says nothing of the credentials");
    // One attempt, then a pause: the reads go to the web site.
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(logins(site), (1, 1), "the login made when linking starts the pause");
    // After the pause one new attempt, which starts another pause.
    p.state.sessions.end_app_backoff(w.user);
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(logins(site), (1, 2));
    // The interface is back: the next attempt obtains the session.
    site.app_login_unavailable.store(false, Ordering::SeqCst);
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(logins(site), (1, 3));
}

#[tokio::test]
async fn a_new_login_to_the_web_site_keeps_the_session_and_its_failure_starts_no_login_to_the_interface() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    asked(site);

    // The web session expires: the account logs in to the web site again. The session of
    // the application interface is another one, and still good: it is kept, not replaced.
    site.expire_sessions();
    let account = p.state.sessions.renew(w.user).await.unwrap();
    assert_eq!(logins(site), (2, 1), "a valid session of the interface was thrown away with the web cookies");
    assert_eq!(account.app_nick().await.unwrap(), "alice");
    assert_eq!(asked(site).1, [call("/auth/me", "app-alice-1")]);
    // It is stored with the new web session.
    assert_eq!(w.start().search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [call("/perfumes", "app-alice-1")]);

    // The web session expires again, and this time the web site does not take the login.
    site.expire_sessions();
    site.login_unavailable.store(true, Ordering::SeqCst);
    assert!(p.state.sessions.renew(w.user).await.is_err());
    assert_eq!(w.link().status, "login_failed");
    // The session of the interface still serves the public reads…
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(w.start().search().await, FROM_THE_INTERFACE, "the failed web login erased the stored session");
    // … until the site ends it. The credentials are then not sent to the interface, a few
    // seconds after the web site would not take them.
    site.app_live.lock().unwrap().clear();
    asked(site);
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(asked(site).1, [call("/perfumes", "app-alice-1")], "a login to the interface during the pause of the web login");
    // Nor once the pause is over, as long as the link waits for the web site.
    p.state.sessions.end_backoff(w.user);
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_WEB);
    assert_eq!(w.start().search().await, FROM_THE_WEB);
    assert_eq!(asked(site).1, []);

    // The web site is back: the next account tool logs in there, then to the interface.
    site.login_unavailable.store(false, Ordering::SeqCst);
    assert!(p.state.sessions.renew(w.user).await.unwrap().has_app_session());
    assert_eq!(asked(site).1, [login()]);
    assert_eq!(w.link().status, "linked");
}

#[tokio::test]
async fn a_token_the_site_refused_does_not_come_back_from_the_database() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    asked(site);

    // Refused to an account tool, which is answered by the web site.
    site.app_live.lock().unwrap().clear();
    assert_eq!(p.own_nick().await, "alice");
    assert_eq!(asked(site).1, [call("/auth/me", "app-alice-1")]);
    // The process ends there. The next one does not find the token: it logs in.
    let p = w.start();
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-2")], "the refused token was stored, and sent again after the restart");

    // Much later (a session renewed and lost at once is not renewed again, restart or not:
    // that rule has its own tests), refused to a tool that takes the account client
    // itself, as the session status does.
    site.app_live.lock().unwrap().clear();
    let account = p.state.sessions.account_client(w.user).await.unwrap();
    assert_eq!(account.own_nick().await.unwrap(), "alice");
    let _ = p.tools.finish(&p.user, "parfumo_session_status", "session", std::time::Instant::now(), 0, Ok((String::new(), json!({}))));
    assert_eq!(asked(site).1, [call("/auth/me", "app-alice-2")]);
    p.state.sessions.end_app_backoff(w.user);
    let p = w.start();
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-3")]);

    // Refused to a call that is abandoned before its end.
    site.app_live.lock().unwrap().clear();
    let caller = Caller(p.user.clone());
    let abandoned = p.tools.run(&caller, "parfumo_search_perfumes", "sauvage", |client| async move {
        let _ = client.app_nick().await;
        std::future::pending::<()>().await;
        Ok((String::new(), json!({})))
    });
    assert!(tokio::time::timeout(Duration::from_millis(300), abandoned).await.is_err(), "the call never ends: it is dropped");
    assert_eq!(asked(site).1, [call("/auth/me", "app-alice-3")]);
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(w.start().search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-4")]);
}

#[tokio::test]
async fn a_login_under_way_when_another_account_is_linked_leaves_nothing_in_the_new_link() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    // The site ended alice's session, which a read found out.
    site.app_live.lock().unwrap().clear();
    assert_eq!(p.search().await, FROM_THE_WEB);
    asked(site);

    // The next read logs in to the interface as alice, and the answer is slow to come.
    // Meanwhile the user links bob's account.
    site.app_delay_ms.store(300, Ordering::SeqCst);
    let linking = async {
        while !site.app_seen.lock().unwrap().contains(&login()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        site.app_delay_ms.store(0, Ordering::SeqCst);
        p.link_to("bob").await;
    };
    let (found, ()) = tokio::join!(p.search(), linking);
    assert_eq!(found, FROM_THE_WEB, "the read that logged in for the previous account has no session to use");
    let alice_login = site.app_logins.load(Ordering::SeqCst) - 1;
    assert!(site.app_live.lock().unwrap().contains_key(&format!("app-alice-{alice_login}")), "the site did issue a token for alice");
    asked(site);

    // What the user's clients hold is bob's session, and bob's alone.
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(p.state.sessions.account_client(w.user).await.unwrap().app_nick().await.unwrap(), "bob");
    // So is what the database holds: the token and the web cookies of bob.
    let restarted = w.start();
    assert_eq!(restarted.search().await, FROM_THE_INTERFACE);
    assert_eq!(restarted.own_nick().await, "bob");
    let (_, interface) = asked(site);
    assert!(!interface.is_empty() && interface.iter().all(|(_, token)| token.starts_with("app-bob-")), "{interface:?}");
    assert_eq!(logins(site).0, 2, "the stored web session is bob's: no new web login after the restart");
}

#[tokio::test]
async fn a_link_that_changes_at_the_instant_a_login_leaves_keeps_nothing_of_the_previous_account() {
    let w = World::new().await;
    let (site, user) = (&w.site, w.user);
    let p = w.start();
    // alice is linked, and the site ended her session: the next read logs in again.
    let lose_the_session = || async {
        p.link_to("alice").await;
        site.app_live.lock().unwrap().clear();
        assert_eq!(p.search().await, FROM_THE_WEB);
        asked(site);
    };

    // The user unlinks the account at the very instant that login leaves: the link was
    // checked, the request is not sent yet.
    lose_the_session().await;
    let sessions = p.state.sessions.clone();
    p.state.sessions.as_the_app_login_leaves(move || assert!(sessions.unlink(user).unwrap()));
    assert_eq!(p.search().await, FROM_THE_WEB, "the read used the token of an account that is no longer linked");
    assert_eq!(asked(site).1, [login()], "the login had left: the site issued a token");
    assert!(!p.holds_session(), "the token of an unlinked account is held until the process ends");
    assert_eq!(p.search().await, FROM_THE_WEB);
    assert_eq!(asked(site).1, []);

    // The same when another account is linked at that instant: the token alice's login
    // obtains is not kept under bob's link.
    lose_the_session().await;
    let sessions = p.state.sessions.clone();
    p.state.sessions.as_the_app_login_leaves(move || {
        let bob = LocalAccount::Credentials(Credentials { username: "bob".into(), password: "pw-bob".into() });
        assert!(sessions.sync_local_link(user, Some(&bob)).unwrap());
    });
    assert_eq!(p.search().await, FROM_THE_WEB, "the read used alice's token under bob's link");
    assert_eq!(asked(site).1, [login()]);
    assert!(!p.holds_session());
    // bob's link logs in for itself, and what the clients and the database hold is bob's.
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(p.own_nick().await, "bob");
    let restarted = w.start();
    assert_eq!(restarted.search().await, FROM_THE_INTERFACE);
    let (_, interface) = asked(site);
    assert_eq!(interface[0], login());
    assert!(interface.len() > 2 && interface[1..].iter().all(|(_, token)| token.starts_with("app-bob-")), "{interface:?}");
    assert!(site.app_calls.lock().unwrap().iter().rev().take(interface.len() - 1).all(|(member, _)| member == "bob"));
}

#[tokio::test]
async fn a_restart_does_not_forget_that_a_renewed_session_was_lost() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    asked(site);

    // The interface accepts every login and refuses every token: one renewal, then a pause.
    site.app_tokens_refused.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(asked(site).1, [call("/perfumes", "app-alice-1"), login(), call("/perfumes", "app-alice-2")]);

    // The program is started again during the pause, as the stdio server is with every
    // session of its MCP client: no login, no refused request, for the tools of either kind.
    let p = w.start();
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(p.own_nick().await, "alice");
    assert_eq!(asked(site).1, [], "a new start logged in again, and had a token refused, during the pause of the previous one");

    // Started again once the pause is over: the site had ended the session, so this login
    // is a renewal too. One, and when its session is lost as well, a pause.
    p.state.sessions.end_app_backoff(w.user);
    let p = w.start();
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-3")], "two logins and two refusals at every start");
    assert_eq!(w.start().search().await, FROM_THE_WEB);
    assert_eq!(asked(site).1, []);

    // Linking the account again starts afresh: a session, and one renewal.
    site.app_tokens_refused.store(false, Ordering::SeqCst);
    p.link_to("alice").await;
    assert_eq!(w.start().search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-4")]);
}

#[tokio::test]
async fn a_login_the_interface_forbids_without_refusing_the_credentials_is_followed_by_a_pause() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();

    // 403 on the login: what the interface answers to a wrong password was never recorded,
    // and this may as well be a lockout or a rule on the address. The credentials are not
    // given up for it.
    site.app_login_forbidden.store(true, Ordering::SeqCst);
    p.link_to("alice").await;
    assert_eq!(logins(site), (1, 1));
    assert!(!w.link().app_refused, "credentials that may be good were refused for good");
    for _ in 0..3 {
        assert_eq!(p.search().await, FROM_THE_WEB);
    }
    assert_eq!(logins(site), (1, 1), "one attempt, then a pause");
    // After the pause, or in a new process, one new attempt.
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_WEB);
    assert_eq!(w.start().search().await, FROM_THE_WEB);
    assert_eq!(logins(site), (1, 3));
    // The interface takes the login again: the session is obtained.
    site.app_login_forbidden.store(false, Ordering::SeqCst);
    p.state.sessions.end_app_backoff(w.user);
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(logins(site), (1, 4));
}

#[tokio::test]
async fn unlinking_or_linking_another_account_takes_the_token_away_from_every_client() {
    let w = World::new().await;
    let site = &w.site;
    let p = w.start();
    p.link_to("alice").await;
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    asked(site);

    // Unlinked: the token of the account is gone for the public client too.
    assert!(p.state.sessions.unlink(w.user).unwrap());
    assert!(!p.holds_session());
    assert_eq!(p.search().await, FROM_THE_WEB);
    // A link by cookie has no credentials: no login to the interface, no token.
    assert!(!p.state.sessions.link_cookie(w.user, "pf_auth=not-a-live-session").await.unwrap());
    assert_eq!(p.search().await, FROM_THE_WEB);
    assert_eq!(asked(site).1, [], "nothing is asked of the interface without a linked account that can log in to it");

    // Another account: its own token, obtained with its own credentials.
    p.link_to("bob").await;
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-bob-2")]);
    // Linked again as the first one: bob's token does not serve alice's link.
    p.link_to("alice").await;
    assert_eq!(p.search().await, FROM_THE_INTERFACE);
    assert_eq!(asked(site).1, [login(), call("/perfumes", "app-alice-3")]);
    assert_eq!(site.app_calls.lock().unwrap().last().unwrap().0, "alice");
}

#[tokio::test]
async fn while_the_address_is_blocked_every_read_goes_to_the_web_site_and_nothing_to_the_interface() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (alice, alice_key) = linked(&app, "alice").await;
    let (bob, bob_key) = linked(&app, "bob").await;
    asked(&site);
    let search = json!({"query": "sauvage"});

    // The site blocks the address. The call that meets the block page is answered by the
    // web site, not by five minutes of errors.
    site.app_blocked.store(true, Ordering::SeqCst);
    let found = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(rows(&found), FROM_THE_WEB, "{found}");
    let (web, interface) = asked(&site);
    assert_eq!(interface, [call("/perfumes", "app-alice-1")]);
    assert!(searched_the_web(&web), "{web:?}");

    // While the block lasts nothing leaves for the interface, for any user of the server.
    for key in [&alice_key, &bob_key, &alice_key, &bob_key] {
        assert_eq!(rows(&app.call_tool(key, "parfumo_search_perfumes", search.clone()).await), FROM_THE_WEB);
    }
    let (web, interface) = asked(&site);
    assert_eq!(interface, [], "a request left for the interface while the address is blocked");
    assert!(searched_the_web(&web), "{web:?}");
    // A block says nothing about the sessions: they are kept for when it is over.
    for user in [alice, bob] {
        assert!(app.state.sessions.public_client_for(user).unwrap().has_app_session());
    }

    // If the block covers the web pages too, the answer asks to wait, where the block page
    // read as a web page would say that the perfume does not exist.
    site.web_blocked.store(true, Ordering::SeqCst);
    let blocked = app.call_tool(&bob_key, "parfumo_fetch_page", json!({"url": "/Perfumes/Dior/Sauvage_Eau_de_Toilette"})).await;
    assert_eq!(error_code(&blocked), "rate_limited", "{blocked}");
    assert!(blocked["content"][0]["text"].as_str().unwrap().contains("seconds before retrying"), "{blocked}");
}
