//! The session of the application interface (`/app/v1`) of a linked account: obtained with
//! the link, kept across a restart, renewed once when the site ends it, never insisted on.

use std::sync::atomic::Ordering;

use parfumo_client::ClientError;

use serde_json::json;

use crate::{
    crypto::MasterKey,
    db::Db,
    state::AppState,
    testing::{TestApp, link_account, linked, ok, site::spawn_site, test_config},
};

#[tokio::test]
async fn a_linked_account_gets_keeps_and_renews_its_session_of_the_application_interface() {
    let (base, site) = spawn_site().await;
    let key = MasterKey::generate_base64();
    let db = Db::open_in_memory().unwrap();
    let state = AppState::new(test_config(&base), db.clone(), MasterKey::from_base64(&key).unwrap());
    let user = db.add_user("alice", "hash").unwrap();

    // Linking logs in to the web site and to the application interface, once each.
    link_account(&state, user, "alice").await;
    assert_eq!((site.logins.load(Ordering::SeqCst), site.app_logins.load(Ordering::SeqCst)), (1, 1));
    let client = state.sessions.account_client(user).await.unwrap();
    assert!(client.has_app_session());
    assert_eq!(client.app_nick().await.unwrap(), "alice");

    // A restart finds the token with the cookies: no new login of either kind.
    let restarted = AppState::new(test_config(&base), db.clone(), MasterKey::from_base64(&key).unwrap());
    let again = restarted.sessions.account_client(user).await.unwrap();
    assert_eq!(again.app_nick().await.unwrap(), "alice");
    assert_eq!((site.logins.load(Ordering::SeqCst), site.app_logins.load(Ordering::SeqCst)), (1, 1));
    // Neither the token nor the password is stored in clear.
    let stored = db.link_get(user).unwrap().unwrap();
    for blob in [stored.cookies.unwrap(), stored.secret.unwrap()] {
        let text = String::from_utf8_lossy(&blob).into_owned();
        assert!(!text.contains("app-alice") && !text.contains("pw-alice"), "a secret is stored in clear");
    }

    // The site ends the session: the refused token is forgotten, and the next check of the
    // account logs in to the application interface again, once, without a web login.
    site.app_live.lock().unwrap().clear();
    assert_eq!(client.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    assert!(!client.has_app_session());
    let renewed = state.sessions.renew(user).await.unwrap();
    assert_eq!(renewed.app_nick().await.unwrap(), "alice");
    assert_eq!((site.logins.load(Ordering::SeqCst), site.app_logins.load(Ordering::SeqCst)), (1, 2));

    // The application interface refuses the login: one attempt, then a pause. The account
    // stays linked, on the web session.
    site.app_live.lock().unwrap().clear();
    site.app_login_refused.store(true, Ordering::SeqCst);
    assert_eq!(renewed.app_nick().await.unwrap_err(), ClientError::LoginRequired);
    for _ in 0..3 {
        let web_only = state.sessions.renew(user).await.unwrap();
        assert!(!web_only.has_app_session());
    }
    assert_eq!(site.app_logins.load(Ordering::SeqCst), 3, "a refused login is not repeated at every call");
    assert_eq!(site.logins.load(Ordering::SeqCst), 1, "the web session is untouched");
    // After the pause, one new attempt.
    site.app_login_refused.store(false, Ordering::SeqCst);
    state.sessions.end_app_backoff(user);
    assert!(state.sessions.renew(user).await.unwrap().has_app_session());
    assert_eq!(site.app_logins.load(Ordering::SeqCst), 4);
}

/// The paths of the web site asked since the last call, and those of the application
/// interface with the account they were asked for.
fn asked(site: &crate::testing::site::Site) -> (Vec<String>, Vec<(String, String)>) {
    let web = site.seen.lock().unwrap().drain(..).map(|(path, _)| path).filter(|p| !p.starts_with("/app/v1")).collect();
    (web, site.app_calls.lock().unwrap().drain(..).collect())
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
    assert_eq!(ok(&found)["items"][0]["id"], "Dior/Sauvage_Eau_de_Toilette", "{found}");
    assert_eq!(ok(&found)["items"][0]["gender"], "men");
    let (web, interface) = asked(&site);
    assert_eq!(interface, [("alice".to_string(), "/perfumes".to_string())]);
    assert!(web.iter().all(|p| p != "/s_perfumes_x.php"), "{web:?}");

    // Not linked: nothing is asked of the interface; the search of the web site is.
    // (The stand-in site has no search page: what matters here is which side is asked.)
    let from_the_web = app.call_tool(&bob_key, "parfumo_search_perfumes", search.clone()).await;
    let (web, interface) = asked(&site);
    assert!(interface.is_empty(), "{interface:?}");
    assert_eq!(web, ["/s_perfumes_x.php"]);

    // The site ends alice's session: the same call is answered by the web site at once,
    // exactly as for a user without an account, with no login in the middle of the call.
    site.app_live.lock().unwrap().clear();
    let logins = site.app_logins.load(Ordering::SeqCst);
    let answer = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(answer, from_the_web);
    let (web, interface) = asked(&site);
    assert!(interface.is_empty(), "the refused call is not one the site served: {interface:?}");
    assert_eq!(web, ["/s_perfumes_x.php"]);
    assert_eq!(site.app_logins.load(Ordering::SeqCst), logins);

    // The account forgot the refused token too: the next call logs in again, once, before
    // reading, and the interface is used again.
    let found = app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await;
    assert_eq!(ok(&found)["items"][0]["id"], "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(site.app_logins.load(Ordering::SeqCst), logins + 1);
    assert_eq!(asked(&site).1, [("alice".to_string(), "/perfumes".to_string())]);

    // When that login is refused, the web site answers, and the login is not tried again
    // at every call.
    site.app_live.lock().unwrap().clear();
    site.app_login_refused.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(app.call_tool(&alice_key, "parfumo_search_perfumes", search.clone()).await, from_the_web);
    }
    assert_eq!(site.app_logins.load(Ordering::SeqCst), logins + 2, "one refused login, then a pause");
    assert!(!app.state.sessions.account_client(alice).await.unwrap().has_app_session());
}
