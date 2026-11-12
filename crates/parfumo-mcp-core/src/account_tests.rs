//! Tests of the collection and wear tools (jalon 2), without network: the tools are called
//! in process against the stand-in for parfumo.com of `testing::site`.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use crate::testing::{TestApp, error_code, linked, ok, site::*, test_config};

#[tokio::test]
async fn the_account_tools_are_listed_with_honest_annotations() {
    let app = TestApp::new("http://127.0.0.1:9");
    let (_, token) = app.add_user("alice");
    let tools = app.list_tools(&token).await;
    assert_eq!(tools.len(), 48);
    let annotations = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name} missing"))["annotations"].clone();
    for (name, read_only, destructive, idempotent) in [
        ("parfumo_list_collections", true, false, true),
        ("parfumo_get_collection", true, false, true),
        ("parfumo_add_to_collection", false, false, true),
        ("parfumo_move_collection_item", false, false, false),
        ("parfumo_update_collection_item", false, true, true),
        ("parfumo_remove_from_collection", false, true, true),
        ("parfumo_manage_custom_collection", false, true, false),
        ("parfumo_export_collection", true, false, true),
        ("parfumo_collection_stats", true, false, true),
        ("parfumo_log_wear", false, false, false),
        ("parfumo_list_wears", true, false, true),
        ("parfumo_delete_wear", false, true, true),
        ("parfumo_rate_perfume", false, true, true),
        ("parfumo_classify_perfume", false, true, true),
        ("parfumo_list_my_ratings", true, false, true),
        ("parfumo_write_review", false, true, true),
        ("parfumo_write_statement", false, true, true),
        ("parfumo_delete_review_or_statement", false, true, true),
        ("parfumo_list_my_reviews", true, false, true),
    ] {
        let a = annotations(name);
        assert_eq!(
            (a["readOnlyHint"].as_bool(), a["destructiveHint"].as_bool(), a["idempotentHint"].as_bool()),
            (Some(read_only), Some(destructive), Some(idempotent)),
            "{name}"
        );
        assert_eq!(a["openWorldHint"], true, "{name}");
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        let has_dry_run = tool["inputSchema"]["properties"].get("dry_run").is_some();
        assert_eq!(has_dry_run, !read_only, "{name}: every write tool, and only them, takes dry_run");
        let d = tool["description"].as_str().unwrap();
        assert!(d.is_ascii() && d.contains("linked Parfumo account"), "{name}");
    }
    let description = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["description"].as_str().unwrap().to_string();
    for public in ["parfumo_log_wear", "parfumo_rate_perfume", "parfumo_classify_perfume", "parfumo_write_review", "parfumo_write_statement"] {
        let d = description(public);
        assert!(d.contains("PUBLIC") && d.contains("dry-run mode unless the operator enabled them"), "{public}");
    }
    for own_words in ["parfumo_write_review", "parfumo_write_statement"] {
        assert!(description(own_words).contains("forbids AI-generated"), "{own_words}");
    }
}

#[tokio::test]
async fn account_tools_need_a_linked_account() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = app.add_user("carol");
    for (tool, args) in [
        ("parfumo_list_collections", json!({})),
        ("parfumo_add_to_collection", json!({"perfume": "111"})),
        ("parfumo_list_wears", json!({})),
        ("parfumo_log_wear", json!({"perfume": "111", "dry_run": true})),
    ] {
        let r = app.call_tool(&token, tool, args).await;
        assert_eq!(error_code(&r), "not_linked", "{tool}");
        // Called in process, the tools name the environment variables of the local binary;
        // the HTTP server names its `/link` page instead (tested there).
        assert!(r["content"][0]["text"].as_str().unwrap().contains("PARFUMO_COOKIE"), "{r}");
    }
    assert!(site.seen.lock().unwrap().is_empty(), "nothing is sent to Parfumo without a linked account");
}

#[tokio::test]
async fn collection_writes_are_idempotent_dry_runnable_and_read_back() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = linked(&app, "alice").await;

    // Seven fixed lists, all empty.
    let r = app.call_tool(&token, "parfumo_list_collections", json!({})).await;
    let lists = ok(&r)["collections"].as_array().unwrap().clone();
    assert_eq!(lists.len(), 7);
    assert_eq!(lists[6]["slug"], "mini");
    assert_eq!(ok(&r)["custom_columns"][0]["name"], "Gift");

    // Dry run: the request is shown, nothing is written.
    let r = app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": SAUVAGE, "dry_run": true})).await;
    let out = ok(&r);
    assert_eq!((out["dry_run"].as_bool(), out["executed"].as_bool()), (Some(true), Some(false)));
    assert_eq!(out["planned_requests"][0]["path"], "/api/wardrobe/wardrobe_add_remove");
    assert_eq!(out["planned_requests"][0]["form"], json!({"p": "63363", "t": "1", "action": "true", "hidden": "No"}));
    assert!(!r.to_string().contains(CSRF), "the CSRF token never appears in a tool answer");
    assert_eq!(site.write_count(), 0);

    // Real add, by id slug: resolved to the numeric id on the public page, state read back.
    let r = app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": SAUVAGE})).await;
    let out = ok(&r);
    assert_eq!(out["status"], "added");
    assert_eq!(out["item"]["perfume_id"], SAUVAGE);
    assert_eq!(out["collection"]["count"], 1);
    let entry_id = out["item"]["entry_id"].as_u64().unwrap();

    // Adding again is not an error and sends nothing.
    let before = site.write_count();
    let r = app
        .call_tool(
            &token,
            "parfumo_add_to_collection",
            json!({"perfume": format!("{base}/Perfumes/Dior/Sauvage_Eau_de_Toilette"), "collection": "I have"}),
        )
        .await;
    assert_eq!(ok(&r)["status"], "already_present");
    assert_eq!(site.write_count(), before);

    // Unknown perfume: Parfumo answers 0.
    let r = app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": "999"})).await;
    assert_eq!(error_code(&r), "not_found");
    let r = app.call_tool(&token, "parfumo_get_collection", json!({"collection": "no such list"})).await;
    assert_eq!(error_code(&r), "not_found");

    // Update: only the given fields change; the four detail fields are merged with the current ones.
    let r = app
        .call_tool(&token, "parfumo_update_collection_item", json!({"perfume": SAUVAGE, "bottle_size": "100", "price_paid": 85, "currency": "eur", "remaining_ml": 60, "private_note": "gift from L.", "custom": {"gift": "1"}}))
        .await;
    let item = &ok(&r)["item"];
    assert_eq!(ok(&r)["status"], "updated");
    assert_eq!((item["bottle_size"].as_str(), item["price_paid"].as_f64(), item["currency"].as_str()), (Some("100"), Some(85.0), Some("EUR")));
    assert_eq!(
        (item["remaining_ml"].as_f64(), item["private_note"].as_str(), item["custom"]["Gift"].as_str()),
        (Some(60.0), Some("gift from L."), Some("1"))
    );
    let r = app.call_tool(&token, "parfumo_update_collection_item", json!({"perfume": SAUVAGE, "quantity": 2})).await;
    let item = &ok(&r)["item"];
    assert_eq!(
        (item["quantity"].as_u64(), item["bottle_size"].as_str(), item["price_paid"].as_f64()),
        (Some(2), Some("100"), Some(85.0)),
        "untouched fields survive"
    );
    let details = site.writes.lock().unwrap().iter().rfind(|(_, p, _)| p == "/api/wardrobe/update_details").unwrap().2.clone();
    assert!(details.contains(&format!("id={entry_id}")) && details.contains("currency=EUR") && details.contains("paid=85"), "{details}");
    for (args, what) in [
        (json!({"perfume": SAUVAGE}), "nothing to change"),
        (json!({"perfume": SAUVAGE, "custom": {"Nope": "1"}}), "unknown custom column"),
        (json!({"perfume": "111", "quantity": 2}), "not in the list"),
        (json!({"perfume": SAUVAGE, "quantity": 0}), "zero quantity"),
    ] {
        let r = app.call_tool(&token, "parfumo_update_collection_item", args).await;
        assert!(matches!(error_code(&r).as_str(), "invalid_input" | "not_found"), "{what}: {r}");
    }

    // Copy to the wish list, then the copy shows there and the source keeps it.
    let r = app.call_tool(&token, "parfumo_move_collection_item", json!({"perfume": SAUVAGE, "from": "have", "to": "wish_list", "copy": true})).await;
    assert_eq!(ok(&r)["status"], "copied");
    assert_eq!(ok(&r)["collection"]["slug"], "want");
    let r = app.call_tool(&token, "parfumo_move_collection_item", json!({"perfume": SAUVAGE, "from": "have", "to": "want"})).await;
    assert_eq!(ok(&r)["status"], "already_in_target");
    let r = app.call_tool(&token, "parfumo_move_collection_item", json!({"perfume": SAUVAGE, "from": "want", "to": "tested"})).await;
    assert_eq!(ok(&r)["status"], "moved");
    assert!(ok(&r)["notes"][0].as_str().unwrap().contains("now holds 0"));

    // Remove from "I have": Parfumo keeps it in "I had", and the tool says so.
    let r = app.call_tool(&token, "parfumo_remove_from_collection", json!({"perfume": SAUVAGE, "dry_run": true})).await;
    assert_eq!(ok(&r)["planned_requests"].as_array().unwrap().len(), 1);
    let r = app.call_tool(&token, "parfumo_remove_from_collection", json!({"perfume": SAUVAGE})).await;
    assert_eq!(ok(&r)["status"], "removed");
    assert!(ok(&r)["notes"].to_string().contains("also_from_had"));
    let r = app.call_tool(&token, "parfumo_get_collection", json!({"collection": "had"})).await;
    assert_eq!(ok(&r)["items"][0]["perfume_id"], SAUVAGE);
    // Absent from "I have" now: with also_from_had the leftover in "I had" is removed.
    let r = app.call_tool(&token, "parfumo_remove_from_collection", json!({"perfume": SAUVAGE, "also_from_had": true})).await;
    assert_eq!(ok(&r)["status"], "removed");
    let r = app.call_tool(&token, "parfumo_get_collection", json!({"collection": "had"})).await;
    assert_eq!(ok(&r)["total_items"], 0);
    // Removing what is not there is not an error and sends nothing.
    let before = site.write_count();
    let r = app.call_tool(&token, "parfumo_remove_from_collection", json!({"perfume": SAUVAGE, "also_from_had": true})).await;
    assert_eq!(ok(&r)["status"], "absent");
    assert_eq!(site.write_count(), before);

    // Every write is in the audit log with its target.
    let audit = app.state.db.audit_list(Some("alice"), 100).unwrap();
    assert!(audit.iter().any(|a| a.tool == "parfumo_remove_from_collection" && a.target.contains("dry_run=false") && a.result == "ok"));
    assert!(audit.iter().any(|a| a.tool == "parfumo_add_to_collection" && a.result == "not_found"));
    // The log tells real writes from dry runs, refusals and reads.
    assert!(
        audit.iter().any(|a| a.tool == "parfumo_remove_from_collection" && a.target.contains("dry_run=false") && a.writes >= 1),
        "a real write counts the requests sent"
    );
    assert!(audit.iter().filter(|a| a.target.contains("dry_run=true")).all(|a| a.writes == 0), "a dry run sends nothing");
    assert!(audit.iter().filter(|a| a.tool == "parfumo_get_collection" || a.tool == "parfumo_list_collections").all(|a| a.writes == 0));
    let sent: i64 = audit.iter().map(|a| a.writes).sum();
    assert_eq!(sent as usize, site.write_count(), "every write request that reached Parfumo is accounted for");
    let only_writes =
        app.state.db.audit_query(&crate::db::AuditFilter { user: Some("alice".into()), writes_only: true, ..Default::default() }, 100).unwrap();
    assert!(!only_writes.is_empty() && only_writes.iter().all(|a| a.writes > 0));
}

#[tokio::test]
async fn custom_lists_are_created_renamed_hidden_and_deleted() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = linked(&app, "alice").await;
    let manage = |args: Value| app.call_tool(&token, "parfumo_manage_custom_collection", args);

    let r = manage(json!({"action": "create", "name": "Summer", "dry_run": true})).await;
    assert_eq!(ok(&r)["planned_requests"].as_array().unwrap().len(), 2);
    assert_eq!(site.write_count(), 0);

    let r = manage(json!({"action": "create", "name": "Summer", "notes": "for hot days"})).await;
    assert_eq!(ok(&r)["status"], "created");
    assert_eq!(
        (ok(&r)["collection"]["name"].as_str(), ok(&r)["collection"]["notes"].as_str(), ok(&r)["collection"]["custom"].as_bool()),
        (Some("Summer"), Some("for hot days"), Some(true))
    );
    let id = ok(&r)["collection"]["id"].as_u64().unwrap();
    let r = manage(json!({"action": "create", "name": "summer"})).await;
    assert_eq!(ok(&r)["status"], "already_exists", "no duplicate list");

    // A custom list is addressable by name.
    let r = app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": "111", "collection": "Summer"})).await;
    assert_eq!(ok(&r)["collection"]["id"].as_u64(), Some(id));

    let r = manage(json!({"action": "rename", "collection": "Summer", "name": "Summer 2027"})).await;
    assert_eq!((ok(&r)["collection"]["name"].as_str(), ok(&r)["collection"]["notes"].as_str()), (Some("Summer 2027"), Some("for hot days")));
    let r = manage(json!({"action": "hide", "collection": id.to_string()})).await;
    assert_eq!((ok(&r)["status"].as_str(), ok(&r)["collection"]["hidden"].as_bool()), (Some("hidden"), Some(true)));
    let before = site.write_count();
    let r = manage(json!({"action": "hide", "collection": id.to_string()})).await;
    assert_eq!(ok(&r)["status"], "unchanged");
    assert_eq!(site.write_count(), before, "the toggle is not sent twice");

    // Built-in lists cannot be renamed or deleted.
    assert_eq!(error_code(&manage(json!({"action": "delete", "collection": "have"})).await), "invalid_input");
    assert_eq!(error_code(&manage(json!({"action": "rename", "collection": "want", "name": "x"})).await), "invalid_input");
    assert_eq!(error_code(&manage(json!({"action": "create", "name": "x".repeat(41)})).await), "invalid_input");

    let r = manage(json!({"action": "delete", "collection": "Summer 2027", "dry_run": true})).await;
    assert!(ok(&r)["notes"][0].as_str().unwrap().contains("1 entries"));
    let r = manage(json!({"action": "delete", "collection": "Summer 2027"})).await;
    assert_eq!(ok(&r)["status"], "deleted");
    let r = manage(json!({"action": "delete", "collection": "Summer 2027"})).await;
    assert_eq!(ok(&r)["status"], "absent");
    assert_eq!(site.writes_to("/api/wardrobe/del_cust_wr"), 1);
}

#[tokio::test]
async fn export_and_stats_read_the_whole_list() {
    let (base, _site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = linked(&app, "alice").await;
    for p in ["111", "222", SAUVAGE] {
        app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": p})).await;
    }
    app.call_tool(
        &token,
        "parfumo_update_collection_item",
        json!({"perfume": "111", "price_paid": 120.5, "currency": "EUR", "private_note": "he said \"wow\""}),
    )
    .await;

    let r = app.call_tool(&token, "parfumo_export_collection", json!({})).await;
    let out = ok(&r);
    assert_eq!((out["format"].as_str(), out["total_items"].as_u64(), out["returned"].as_u64()), (Some("json"), Some(3), Some(3)));
    assert_eq!(out["items"][0]["notes"], json!(["Ambrox", "Vetiver"]), "the export keeps every field");
    assert!(out.get("next_cursor").is_none());

    let r = app.call_tool(&token, "parfumo_export_collection", json!({"format": "csv"})).await;
    let csv = ok(&r)["csv"].as_str().unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("\"Perfume\",\"Brand\"") && lines[0].contains("\"Gift\""));
    assert!(lines[1].starts_with("\"Bleu de Chanel\",\"Chanel\"") && lines[1].contains("\"120.5\",\"EUR\",\"he said \"\"wow\"\"\""), "{}", lines[1]);

    let r = app.call_tool(&token, "parfumo_export_collection", json!({"format": "csv_site"})).await;
    let csv = ok(&r)["csv"].as_str().unwrap();
    assert!(csv.starts_with("\"Perfume\",\"Brand\",\"Year\""), "BOM removed: {csv}");
    assert_eq!(csv.lines().count(), 4);

    let r = app.call_tool(&token, "parfumo_collection_stats", json!({"top": 2})).await;
    let s = ok(&r);
    assert_eq!((s["total_items"].as_u64(), s["distinct_brands"].as_u64(), s["never_worn_count"].as_u64()), (Some(3), Some(3), Some(3)));
    assert_eq!(s["notes"][0], json!({"name": "Ambrox", "count": 3}));
    assert_eq!(s["value"][0], json!({"currency": "EUR", "total_paid": 120.5, "priced_items": 1, "average_paid": 120.5}));
    assert_eq!(s["items_without_price"], 2);
    assert_eq!(s["brands"].as_array().unwrap().len(), 2, "rankings are cut at `top`");

    // Compact listing by default, paginated.
    let r = app.call_tool(&token, "parfumo_get_collection", json!({"limit": 2})).await;
    let out = ok(&r);
    assert_eq!(out["items"].as_array().unwrap().len(), 2);
    assert!(out["items"][0].get("notes").is_none());
    let r = app.call_tool(&token, "parfumo_get_collection", json!({"limit": 2, "cursor": out["next_cursor"], "include_details": true})).await;
    assert_eq!(ok(&r)["items"].as_array().unwrap().len(), 1);
    assert_eq!(ok(&r)["items"][0]["accords"], json!(["Fresh"]));
    assert!(ok(&r).get("next_cursor").is_none());
}

#[tokio::test]
async fn public_wear_stays_a_dry_run_until_enabled() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, token) = linked(&app, "alice").await;

    // Public writes are off by default: even without dry_run nothing is sent.
    let r = app.call_tool(&token, "parfumo_log_wear", json!({"perfume": SAUVAGE, "date": "2025-01-15", "comment": "office"})).await;
    let out = ok(&r);
    assert_eq!((out["dry_run"].as_bool(), out["executed"].as_bool(), out["public_writes_enabled"].as_bool()), (Some(true), Some(false), Some(false)));
    let paths: Vec<&str> = out["planned_requests"].as_array().unwrap().iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["/api/assistant/i_wear", "/api/account/update_iwear_date", "/api/account/update_iwear_comment", "/api/assistant/i_wear"]);
    assert_eq!(out["planned_requests"][1]["form"]["new_date"], "2025-01-15T12:00");
    assert_eq!(out["planned_requests"][3]["form"]["p"], "clear", "a past wear does not stay displayed as worn now");
    assert!(out["notes"][0].as_str().unwrap().contains("PARFUMO_MCP_PUBLIC_WRITES"));
    assert_eq!(site.write_count(), 0);
    assert_eq!(error_code(&app.call_tool(&token, "parfumo_log_wear", json!({"perfume": SAUVAGE, "date": "2999-01-01"})).await), "invalid_input");
}

#[tokio::test]
async fn wears_are_logged_listed_and_deleted_when_public_writes_are_enabled() {
    let (base, site) = spawn_site().await;
    let mut cfg = test_config(&base);
    cfg.public_writes = true;
    let app = TestApp::with_config(cfg);
    let (_, token) = linked(&app, "alice").await;

    let r = app.call_tool(&token, "parfumo_log_wear", json!({"perfume": SAUVAGE, "dry_run": true})).await;
    assert_eq!((ok(&r)["dry_run"].as_bool(), ok(&r)["public_writes_enabled"].as_bool()), (Some(true), Some(true)));
    assert_eq!(site.write_count(), 0);

    let r = app.call_tool(&token, "parfumo_log_wear", json!({"perfume": SAUVAGE, "date": "2025-01-15T18:30", "comment": "dinner"})).await;
    let out = ok(&r);
    assert_eq!(out["status"], "logged");
    assert_eq!(
        (out["wear"]["worn_at"].as_str(), out["wear"]["comment"].as_str(), out["wear"]["perfume_id"].as_str()),
        (Some("2025-01-15T18:30"), Some("dinner"), Some(SAUVAGE))
    );
    let wear_id = out["wear"]["wear_id"].as_u64().unwrap();
    assert_eq!(site.writes_to("/api/assistant/i_wear"), 2, "log, then clear the 'worn now' display of a past wear");
    app.call_tool(&token, "parfumo_log_wear", json!({"perfume": "111", "date": "2025-03-02"})).await;

    // Period filter: one request per month, bounds inclusive, newest first.
    let r = app.call_tool(&token, "parfumo_list_wears", json!({"from": "2025-01-01", "to": "2025-03-31"})).await;
    let out = ok(&r);
    assert_eq!(out["total_in_period"], 2);
    assert_eq!(out["wears"][0]["worn_at"], "2025-03-02T12:00");
    assert_eq!(out["wears"][1]["wear_id"].as_u64(), Some(wear_id));
    let r = app.call_tool(&token, "parfumo_list_wears", json!({"from": "2025-01-16", "to": "2025-02-28"})).await;
    assert_eq!(ok(&r)["total_in_period"], 0);
    assert_eq!(error_code(&app.call_tool(&token, "parfumo_list_wears", json!({"from": "2023-01-01", "to": "2025-01-01"})).await), "invalid_input");
    assert_eq!(error_code(&app.call_tool(&token, "parfumo_list_wears", json!({"from": "15/01/2025"})).await), "invalid_input");

    let r = app.call_tool(&token, "parfumo_delete_wear", json!({"wear_id": wear_id, "dry_run": true})).await;
    assert_eq!(ok(&r)["wear"]["wear_id"].as_u64(), Some(wear_id));
    assert_eq!(site.writes_to("/api/account/delete_iwear"), 0);
    let r = app.call_tool(&token, "parfumo_delete_wear", json!({"wear_id": wear_id})).await;
    assert_eq!(ok(&r)["status"], "deleted");
    let r = app.call_tool(&token, "parfumo_delete_wear", json!({"wear_id": wear_id})).await;
    assert_eq!(ok(&r)["status"], "absent", "deleting twice is not an error");
    let r = app.call_tool(&token, "parfumo_list_wears", json!({"from": "2025-01-01", "to": "2025-03-31"})).await;
    assert_eq!(ok(&r)["total_in_period"], 1);
}

#[tokio::test]
async fn two_users_never_touch_each_others_account() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (_, alice) = linked(&app, "alice").await;
    let (_, bob) = linked(&app, "bob").await;

    let r = app.call_tool(&alice, "parfumo_add_to_collection", json!({"perfume": "111"})).await;
    let alice_entry = ok(&r)["item"]["entry_id"].as_u64().unwrap();
    app.call_tool(&bob, "parfumo_add_to_collection", json!({"perfume": "222", "collection": "want"})).await;

    // Each sees only their own lists.
    let r = app.call_tool(&bob, "parfumo_get_collection", json!({})).await;
    assert_eq!(ok(&r)["total_items"], 0);
    let r = app.call_tool(&alice, "parfumo_get_collection", json!({})).await;
    assert_eq!(ok(&r)["items"][0]["p_id"], 111);
    let r = app.call_tool(&alice, "parfumo_get_collection", json!({"collection": "want"})).await;
    assert_eq!(ok(&r)["total_items"], 0);

    // Bob cannot update or remove Alice's bottle: the tool only acts on entries of his own lists.
    let before = site.write_count();
    let r = app.call_tool(&bob, "parfumo_update_collection_item", json!({"perfume": "111", "quantity": 9})).await;
    assert_eq!(error_code(&r), "not_found");
    let r = app.call_tool(&bob, "parfumo_remove_from_collection", json!({"perfume": "111"})).await;
    assert_eq!(ok(&r)["status"], "absent");
    assert_eq!(site.write_count(), before, "no write left the server");
    assert!(!site.writes.lock().unwrap().iter().any(|(user, _, body)| user == "bob" && body.contains(&alice_entry.to_string())));

    // Each write reached Parfumo under its owner's session only.
    let writes = site.writes.lock().unwrap().clone();
    assert!(writes.iter().any(|(u, _, b)| u == "alice" && b.contains("p=111")));
    assert!(writes.iter().all(|(u, _, b)| !(u == "alice" && b.contains("p=222"))));
    // The public perfume page is fetched without any account cookie.
    app.call_tool(&alice, "parfumo_add_to_collection", json!({"perfume": SAUVAGE, "dry_run": true})).await;
    let seen = site.seen.lock().unwrap().clone();
    assert!(seen.iter().filter(|(p, _)| p.starts_with("/Perfumes/")).all(|(_, cookie)| !cookie.contains("pf_auth")));
    // Audit trails are separate.
    let audit = app.state.db.audit_list(Some("bob"), 100).unwrap();
    assert!(!audit.is_empty() && audit.iter().all(|a| a.user_name == "bob"));
}

#[tokio::test]
async fn an_expired_session_is_renewed_and_the_call_replayed() {
    let (base, site) = spawn_site().await;
    let app = TestApp::new(&base);
    let (id, token) = linked(&app, "alice").await;
    app.call_tool(&token, "parfumo_add_to_collection", json!({"perfume": "111"})).await;
    assert_eq!(site.logins.load(Ordering::SeqCst), 1);

    // Parfumo forgets the session: the next call logs in again by itself and succeeds.
    site.expire_sessions();
    let r = app.call_tool(&token, "parfumo_get_collection", json!({})).await;
    assert_eq!(ok(&r)["total_items"], 1);
    assert_eq!(site.logins.load(Ordering::SeqCst), 2);

    // Same on a write, whose first request is answered 401 by /api.
    site.expire_sessions();
    let r = app.call_tool(&token, "parfumo_update_collection_item", json!({"perfume": "111", "quantity": 3})).await;
    assert_eq!(ok(&r)["item"]["quantity"], 3);
    assert_eq!(site.logins.load(Ordering::SeqCst), 3);

    // Password changed on Parfumo: reauth_required, and no secret in the answer.
    site.expire_sessions();
    site.accounts.lock().unwrap().remove("alice");
    let r = app.call_tool(&token, "parfumo_list_collections", json!({})).await;
    assert_eq!(error_code(&r), "reauth_required");
    assert!(!r.to_string().contains("pw-alice"));
    assert_eq!(app.state.db.link_get(id).unwrap().unwrap().status, "reauth_required");

    // The refused login is not replayed by the next tool calls: nothing reaches Parfumo.
    let logins = site.logins.load(Ordering::SeqCst);
    for tool in ["parfumo_list_collections", "parfumo_session_status"] {
        let _ = app.call_tool(&token, tool, json!({})).await;
    }
    let r = app.call_tool(&token, "parfumo_list_collections", json!({})).await;
    assert_eq!(error_code(&r), "reauth_required");
    assert_eq!(site.logins.load(Ordering::SeqCst), logins, "refused credentials must not be sent again");
}
