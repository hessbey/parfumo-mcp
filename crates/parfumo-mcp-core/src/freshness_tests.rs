//! Nothing read from Parfumo is kept: the same question asked twice is asked of the site twice.

use serde_json::json;

use crate::testing::{TestApp, ok, site::spawn_site};

#[tokio::test]
async fn the_same_public_page_is_asked_of_the_site_at_every_call() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, key) = app.add_user("alice");
    let page = "/Perfumes/Dior/Sauvage_Eau_de_Toilette";
    for _ in 0..3 {
        let answer = app.call_tool(&key, "parfumo_fetch_page", json!({"url": page})).await;
        assert!(ok(&answer)["untrusted_page_content"]["text"].as_str().unwrap().contains("Sauvage"), "{answer}");
    }
    let asked = site.seen.lock().unwrap().iter().filter(|(path, _)| path == page).count();
    assert_eq!(asked, 3, "an answer is never an old copy");
}
