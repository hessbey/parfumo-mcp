//! The session of the application interface (`/app/v1`) of a linked account: obtained with
//! the link, kept across a restart, renewed once when the site ends it, never insisted on.

use std::sync::atomic::Ordering;

use parfumo_client::ClientError;

use crate::{
    crypto::MasterKey,
    db::Db,
    state::AppState,
    testing::{link_account, site::spawn_site, test_config},
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
