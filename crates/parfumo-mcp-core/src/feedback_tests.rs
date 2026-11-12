//! Integration tests of the rating, classification, review and statement tools (jalon 3),
//! without network, against the stand-in for parfumo.com of `account_tests`.
//!
//! The stand-in follows what was measured for ratings and classification (DISCOVERY §3.4).
//! For reviews and statements it follows the site's forms and scripts (§3.5, §11): those
//! routes have never been sent to the real site.

use serde_json::json;

use crate::testing::{
    TestApp, error_code, linked, ok,
    site::{SAUVAGE, SAUVAGE_P, delete_token, spawn_site},
    test_config,
};

async fn open_app(base: &str) -> TestApp {
    let mut cfg = test_config(base);
    cfg.public_writes = true;
    TestApp::with_config(cfg)
}

#[tokio::test]
async fn public_feedback_stays_a_dry_run_until_enabled() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = linked(&app, "alice").await;

    for (tool, args, path) in [
        ("parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 8.5}), "/api/perfume/add_vote"),
        ("parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["Fresh"]}), "/api/perfume/toggle_classification_vote"),
        ("parfumo_write_review", json!({"perfume": SAUVAGE, "title": "My title", "text": "My own words about it."}), "/api/review/create_review"),
        ("parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "My own short words."}), "/api/statement/create_statement"),
    ] {
        let r = app.call_tool(&token, tool, args).await;
        let out = ok(&r);
        assert_eq!(
            (out["dry_run"].as_bool(), out["executed"].as_bool(), out["public_writes_enabled"].as_bool()),
            (Some(true), Some(false), Some(false)),
            "{tool}"
        );
        assert_eq!(out["status"], "dry_run", "{tool}");
        assert_eq!(out["planned_requests"][0]["path"], path, "{tool}");
        assert_eq!(out["p_id"].as_u64(), Some(SAUVAGE_P), "{tool}");
        assert!(out["notes"][0].as_str().unwrap().contains("PARFUMO_MCP_PUBLIC_WRITES"), "{tool}");
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Dry run: nothing was sent"), "{tool}");
    }
    assert_eq!(site.write_count(), 0, "no publicly visible write leaves the server while the gate is closed");

    // The planned requests are the real payloads.
    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 8.5, "longevity": 7})).await;
    let planned = ok(&r)["planned_requests"].as_array().unwrap().clone();
    assert_eq!(planned.len(), 2);
    assert_eq!(planned[0]["form"], json!({"type": "scent", "p_id": "63363", "v": "85"}));
    assert_eq!(planned[1]["form"]["type"], "durability");
    let r = app.call_tool(&token, "parfumo_write_review", json!({"perfume": SAUVAGE, "title": " T ", "text": " Body "})).await;
    assert_eq!(ok(&r)["planned_requests"][0]["form"], json!({"id": "63363", "title": "T", "text": "Body", "yt": "", "paste_html": ""}));
    assert!(ok(&r)["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("forbids AI-generated")));

    // Reads are not held back.
    let r = app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE})).await;
    assert_eq!(ok(&r)["status"], "read");
    assert_eq!(ok(&r)["my_classification"][0], json!({"group": "Type", "voted": [], "options": ["Fresh", "Woody", "Spicy"]}));
    assert_eq!(ok(&app.call_tool(&token, "parfumo_list_my_ratings", json!({})).await)["items"], json!([]));
    assert_eq!(ok(&app.call_tool(&token, "parfumo_list_my_reviews", json!({"kind": "statement"})).await)["items"], json!([]));
    assert_eq!(site.write_count(), 0);
}

#[tokio::test]
async fn feedback_tools_need_a_linked_account() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = app.add_user("carol");
    for (tool, args) in [
        ("parfumo_rate_perfume", json!({"perfume": "111", "scent": 7})),
        ("parfumo_classify_perfume", json!({"perfume": "111"})),
        ("parfumo_list_my_ratings", json!({})),
        ("parfumo_write_review", json!({"perfume": "111", "title": "t", "text": "x"})),
        ("parfumo_write_statement", json!({"perfume": "111", "text": "x"})),
        ("parfumo_delete_review_or_statement", json!({"perfume": "111", "kind": "review"})),
        ("parfumo_list_my_reviews", json!({})),
    ] {
        assert_eq!(error_code(&app.call_tool(&token, tool, args).await), "not_linked", "{tool}");
    }
    assert!(site.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ratings_are_set_replaced_removed_and_listed() {
    let (base, site) = spawn_site().await;
    let app = open_app(&base).await;
    let (_, token) = linked(&app, "alice").await;

    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 8.5, "value_for_money": 6, "dry_run": true})).await;
    assert_eq!((ok(&r)["dry_run"].as_bool(), ok(&r)["public_writes_enabled"].as_bool()), (Some(true), Some(true)));
    assert_eq!(site.write_count(), 0);

    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 8.5, "value_for_money": 6})).await;
    let out = ok(&r);
    assert_eq!((out["status"].as_str(), out["executed"].as_bool()), (Some("rated"), Some(true)));
    assert_eq!(out["my_ratings"], json!({"scent": 8.5, "pricing": 6.0}), "read back from the rating form");
    assert_eq!(site.writes_to("/api/perfume/add_vote"), 2);

    // Same values again: nothing is sent. One changed value: one request.
    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 8.5})).await;
    assert_eq!((ok(&r)["status"].as_str(), ok(&r)["executed"].as_bool()), (Some("unchanged"), Some(false)));
    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE_P.to_string(), "scent": 9, "value_for_money": 6})).await;
    assert_eq!(ok(&r)["my_ratings"]["scent"], 9.0);
    assert_eq!(site.writes_to("/api/perfume/add_vote"), 3);

    let r = app.call_tool(&token, "parfumo_list_my_ratings", json!({"scale": "scent"})).await;
    let out = ok(&r);
    assert_eq!(out["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        (out["items"][0]["perfume_id"].as_str(), out["items"][0]["rating"].as_f64(), out["items"][0]["brand"].as_str()),
        (Some(SAUVAGE), Some(9.0), Some("Dior"))
    );
    assert_eq!(ok(&app.call_tool(&token, "parfumo_list_my_ratings", json!({"scale": "sillage"})).await)["items"], json!([]));
    let on_ratings: Vec<String> = site.seen.lock().unwrap().iter().map(|(p, _)| p.clone()).filter(|p| p.contains("/Ratings/")).collect();
    assert_eq!(
        on_ratings,
        ["/Users/alice/Ratings/scent", "/Users/alice/Ratings/sillage"],
        "the member's own page, named after the nickname read from the session"
    );

    let r = app.call_tool(&token, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "remove": ["scent", "bottle"]})).await;
    let out = ok(&r);
    assert_eq!(out["status"], "rated");
    assert_eq!(out["my_ratings"], json!({"pricing": 6.0}));
    assert_eq!(site.writes_to("/api/perfume/remove_vote"), 1, "a scale that was not rated is not removed");

    for bad in [
        json!({"perfume": SAUVAGE}),
        json!({"perfume": SAUVAGE, "scent": 7.3}),
        json!({"perfume": SAUVAGE, "bottle": 11}),
        json!({"perfume": SAUVAGE, "scent": 7, "remove": ["scent"]}),
    ] {
        assert_eq!(error_code(&app.call_tool(&token, "parfumo_rate_perfume", bad.clone()).await), "invalid_input", "{bad}");
    }
}

#[tokio::test]
async fn classification_votes_are_added_and_removed_once() {
    let (base, site) = spawn_site().await;
    let app = open_app(&base).await;
    let (_, token) = linked(&app, "alice").await;
    let toggle = "/api/perfume/toggle_classification_vote";

    let r = app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["fresh", "Occasion: night out", "Fresh"]})).await;
    let out = ok(&r);
    assert_eq!(out["status"], "classified");
    assert_eq!(out["my_classification"][0]["voted"], json!(["Fresh"]));
    assert_eq!(out["my_classification"][2], json!({"group": "Occasion", "voted": ["Night Out"], "options": ["Daily", "Night Out"]}));
    assert_eq!(site.writes_to(toggle), 2, "a label given twice is sent once");
    assert_eq!(out["planned_requests"][1]["form"], json!({"p": "63363", "c": "2", "ct": "7", "action": "add"}));

    // Already voted: nothing sent. The site's route is a toggle, so this matters.
    let r = app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["Fresh"]})).await;
    assert_eq!(ok(&r)["status"], "unchanged");
    assert_eq!(site.writes_to(toggle), 2);

    let r = app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["Spring"], "remove": ["Fresh", "Woody"]})).await;
    let out = ok(&r);
    assert_eq!(out["status"], "classified");
    assert_eq!((out["my_classification"][0]["voted"].clone(), out["my_classification"][1]["voted"].clone()), (json!([]), json!(["Spring"])));
    assert_eq!(site.writes_to(toggle), 4, "removing a label that was not voted sends nothing");

    let r = app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["Rainy"]})).await;
    assert_eq!(error_code(&r), "invalid_input");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("Season (Spring, Summer)"));
    assert_eq!(
        error_code(&app.call_tool(&token, "parfumo_classify_perfume", json!({"perfume": SAUVAGE, "add": ["Daily"], "remove": ["daily"]})).await),
        "invalid_input"
    );
}

#[tokio::test]
async fn statements_and_reviews_are_published_replaced_listed_and_deleted() {
    let (base, site) = spawn_site().await;
    let app = open_app(&base).await;
    let (_, token) = linked(&app, "alice").await;

    // Statement: publish, same text again, replace.
    let r = app.call_tool(&token, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "Fresh and loud."})).await;
    let out = ok(&r);
    assert_eq!((out["status"].as_str(), out["my_text"]["text"].as_str()), (Some("published"), Some("Fresh and loud.")));
    let r = app.call_tool(&token, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": " Fresh  and loud. "})).await;
    assert_eq!(ok(&r)["status"], "unchanged");
    let r = app.call_tool(&token, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "Fresh, loud, everywhere."})).await;
    assert_eq!(ok(&r)["status"], "updated");
    assert!(ok(&r)["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("it is replaced")));
    assert_eq!(site.writes_to("/api/statement/create_statement"), 2);

    assert_eq!(
        error_code(&app.call_tool(&token, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "x".repeat(141)})).await),
        "invalid_input"
    );
    assert_eq!(error_code(&app.call_tool(&token, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "  "})).await), "invalid_input");
    let refused = app.call_tool(&token, "parfumo_write_statement", json!({"perfume": "111", "text": "REFUSE"})).await;
    assert_eq!(error_code(&refused), "invalid_input");
    assert!(refused["content"][0]["text"].as_str().unwrap().contains("Please write a real statement."), "the site's own reason is passed on");

    // Review.
    let r = app
        .call_tool(&token, "parfumo_write_review", json!({"perfume": SAUVAGE, "title": "Blue and bright", "text": "First line.\nSecond line."}))
        .await;
    let out = ok(&r);
    assert_eq!((out["status"].as_str(), out["my_text"]["title"].as_str()), (Some("published"), Some("Blue and bright")));
    assert!(out["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("never been exercised")));
    assert_eq!(
        error_code(&app.call_tool(&token, "parfumo_write_review", json!({"perfume": SAUVAGE, "title": "", "text": "x"})).await),
        "invalid_input"
    );
    assert_eq!(
        error_code(&app.call_tool(&token, "parfumo_write_review", json!({"perfume": SAUVAGE, "title": "t", "text": "x".repeat(7501)})).await),
        "invalid_input"
    );

    // Lists.
    let r = app.call_tool(&token, "parfumo_list_my_reviews", json!({})).await;
    let out = ok(&r);
    assert_eq!((out["kind"].as_str(), out["items"].as_array().unwrap().len()), (Some("review"), 1));
    assert_eq!(
        (out["items"][0]["perfume_id"].as_str(), out["items"][0]["title"].as_str(), out["items"][0]["brand"].as_str()),
        (Some(SAUVAGE), Some("Blue and bright"), Some("Dior"))
    );
    let review_id = out["items"][0]["id"].as_u64().unwrap();
    let r = app.call_tool(&token, "parfumo_list_my_reviews", json!({"kind": "statement"})).await;
    assert_eq!(ok(&r)["items"][0]["text"], "Fresh, loud, everywhere.");
    let statement_id = ok(&r)["items"][0]["id"].as_u64().unwrap();

    // Delete: the token of the form's button is sent, never shown.
    let r = app.call_tool(&token, "parfumo_delete_review_or_statement", json!({"perfume": SAUVAGE, "kind": "statement", "dry_run": true})).await;
    assert_eq!(ok(&r)["planned_requests"][0]["form"]["id"], statement_id.to_string());
    assert!(!r.to_string().contains(&delete_token(statement_id)), "the delete token stays on the server");
    assert_eq!(site.writes_to("/api/statement/delete_statement"), 0);
    let r = app.call_tool(&token, "parfumo_delete_review_or_statement", json!({"perfume": SAUVAGE, "kind": "statement"})).await;
    assert_eq!((ok(&r)["status"].as_str(), ok(&r)["my_text"]["text"].as_str()), (Some("deleted"), Some("")));
    assert!(!r.to_string().contains(&delete_token(statement_id)));
    let sent = site.writes.lock().unwrap().iter().find(|(_, p, _)| p == "/api/statement/delete_statement").map(|(_, _, body)| body.clone()).unwrap();
    assert!(sent.contains(&format!("h={}", delete_token(statement_id))), "{sent}");
    let r = app.call_tool(&token, "parfumo_delete_review_or_statement", json!({"perfume": SAUVAGE, "kind": "statement"})).await;
    assert_eq!(ok(&r)["status"], "absent", "deleting twice is not an error");
    assert_eq!(site.writes_to("/api/statement/delete_statement"), 1);

    // The review form of the stand-in only shows its button when asked with the review id:
    // the id is found on the member's own reviews page.
    let r = app.call_tool(&token, "parfumo_delete_review_or_statement", json!({"perfume": SAUVAGE, "kind": "review"})).await;
    assert_eq!(ok(&r)["status"], "deleted");
    assert_eq!(ok(&r)["planned_requests"][0]["form"]["id"], review_id.to_string());
    assert_eq!(ok(&app.call_tool(&token, "parfumo_list_my_reviews", json!({})).await)["items"], json!([]));

    // Without the perfume's page name the id cannot be looked up: nothing is sent.
    app.call_tool(&token, "parfumo_write_review", json!({"perfume": "111", "title": "Second", "text": "Another one."})).await;
    let r = app.call_tool(&token, "parfumo_delete_review_or_statement", json!({"perfume": "111", "kind": "review"})).await;
    assert_eq!(error_code(&r), "upstream_error");
    assert_eq!(site.writes_to("/api/review/delete_review"), 1);
}

#[tokio::test]
async fn feedback_of_two_users_stays_apart_and_is_audited_without_the_text() {
    let (base, site) = spawn_site().await;
    let app = open_app(&base).await;
    let (_, alice) = linked(&app, "alice").await;
    let (_, bob) = linked(&app, "bob").await;

    app.call_tool(&alice, "parfumo_rate_perfume", json!({"perfume": SAUVAGE, "scent": 9})).await;
    app.call_tool(&alice, "parfumo_write_statement", json!({"perfume": SAUVAGE, "text": "Alice's secret opinion."})).await;

    assert_eq!(ok(&app.call_tool(&bob, "parfumo_list_my_ratings", json!({})).await)["items"], json!([]));
    assert_eq!(ok(&app.call_tool(&bob, "parfumo_list_my_reviews", json!({"kind": "statement"})).await)["items"], json!([]));
    let r = app.call_tool(&bob, "parfumo_delete_review_or_statement", json!({"perfume": SAUVAGE, "kind": "statement"})).await;
    assert_eq!(ok(&r)["status"], "absent");
    assert!(site.writes.lock().unwrap().iter().all(|(account, _, _)| account == "alice"));
    assert_eq!(
        ok(&app.call_tool(&alice, "parfumo_list_my_reviews", json!({"kind": "statement"})).await)["items"][0]["text"],
        "Alice's secret opinion."
    );

    // The audit trail names the perfume and the size of the text, not the text.
    let audit = app.state.db.audit_list(None, 50).expect("audit entries");
    let statement = audit.iter().find(|e| e.tool == "parfumo_write_statement").expect("audited");
    assert!(statement.target.contains(SAUVAGE) && statement.target.contains("chars") && !statement.target.contains("secret"), "{}", statement.target);
}
