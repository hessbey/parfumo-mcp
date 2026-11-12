//! Fixture tests of the jalon 4 parsers (profile, member collection, mailbox,
//! notifications, follows, settings, write forms). No network.

use parfumo_client::{ClientError, parse};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn assert_outdated<T: std::fmt::Debug>(r: Result<T, ClientError>) {
    assert!(matches!(r, Err(ClientError::ParserOutdated(_))), "{r:?}");
}

#[test]
fn public_profile_of_an_active_member() {
    let p = parse::parse_member_profile(&fixture("user_profile.html"), "https://www.parfumo.com/Users/member273").unwrap();
    assert_eq!((p.nick.as_str(), p.member_id, p.gender.as_deref()), ("member273", Some(6467), Some("men")));
    assert_eq!((p.rank.as_deref(), p.country.as_deref()), (Some("Club 10"), Some("Pitcairn")));
    assert!(p.activity.as_deref().unwrap().starts_with("Active "));
    assert_eq!((p.points, p.followers, p.member_since.as_deref()), (Some(647_687), Some(806), Some("01/10/2012")));
    let counter = |label: &str| p.counters.iter().find(|c| c.label == label).map(|c| c.count);
    assert_eq!((counter("Collections"), counter("Reviews"), counter("Statements"), counter("Wall")), (Some(300), Some(417), Some(10_129), Some(10)));
    let trophies = |label: &str| p.achievements.iter().find(|c| c.label == label).map(|c| c.count);
    assert_eq!((trophies("Reviews"), trophies("Statements"), trophies("Blog posts")), (Some(18_201), Some(161_261), Some(3_225)));
    assert_eq!(p.signature_scent.as_ref().map(|s| s.id.as_str()), Some("Dior/Eau_Sauvage_Eau_de_Toilette"));
    assert!(p.wearing.is_none());
    assert_eq!((p.collections[0].id.as_str(), p.collections[0].name.as_str(), p.collections[0].count), ("I_have", "I have", Some(300)));
    let custom = p.collections.iter().find(|l| l.id == "1922").unwrap();
    assert_eq!((custom.name.as_str(), custom.count), ("Vetiver d'Orient", Some(28)));
    assert_outdated(parse::parse_member_profile("<html><body><h1>x</h1></body></html>", "u"));
}

#[test]
fn profiles_seen_logged_in() {
    let other = parse::parse_member_profile(&fixture("auth_profile_other.html"), "https://www.parfumo.com/Users/otheruser").unwrap();
    assert_eq!(
        (other.nick.as_str(), other.member_id, other.rank.as_deref(), other.points),
        ("otheruser", Some(1_000_002), Some("New user"), Some(0))
    );
    assert_eq!(other.activity.as_deref(), Some("Active 8 hours ago"));
    assert_eq!(other.wearing.as_ref().map(|w| w.id.as_str()), Some("Giorgio_Armani/Acqua_di_Gio_pour_Homme_Eau_de_Toilette"));
    assert_eq!(other.collections.iter().map(|l| (l.id.as_str(), l.count)).collect::<Vec<_>>(), [("I_have", Some(13)), ("Wish_List", Some(9))]);
    assert_eq!(other.counters.iter().map(|c| (c.label.as_str(), c.count)).collect::<Vec<_>>(), [("Collections", 13), ("Wall", 0)]);

    let own = parse::parse_member_profile(&fixture("auth_profile_own.html"), "https://www.parfumo.com/Users/testuser").unwrap();
    assert_eq!((own.nick.as_str(), own.member_id, own.activity.as_deref(), own.points), ("testuser", Some(1_000_001), Some("Online"), Some(0)));
    assert_eq!(own.member_since.as_deref(), Some("01/01/2026"));
    assert!(own.collections.is_empty() && own.signature_scent.is_none());
}

#[test]
fn collection_list_of_another_member() {
    let call = parse::parse_wardrobe_call(&fixture("user_collection_list.html")).unwrap();
    assert_eq!(
        (call.member_id, call.nick.as_str(), call.list_type.as_str(), call.type_url.as_str(), call.total.as_str()),
        (6467, "member273", "memo", "Watch_List", "9")
    );
    let items = parse::parse_member_wardrobe(&fixture("frag_user_wardrobe.html")).unwrap();
    assert_eq!(items.len(), 9);
    assert_eq!((items[2].p_id, items[2].entry_id, items[2].name.as_str()), (16_624, 33_060_173, "Robin Hood Eau de Cologne"));
    assert!(items[0].image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    assert_outdated(parse::parse_wardrobe_call("<html></html>"));
    assert_outdated(parse::parse_member_wardrobe("<div>nothing</div>"));

    let (id, name, brand, url) = parse::parse_wardrobe_peek(&fixture("auth_frag_wardrobe_sneak_peek.html")).unwrap();
    assert_eq!((id.as_str(), name.as_str(), brand.as_deref()), ("Dior/Sauvage_Eau_de_Toilette", "Sauvage Eau de Toilette", Some("Dior")));
    assert_eq!(url, "https://www.parfumo.com/Perfumes/Dior/Sauvage_Eau_de_Toilette");
}

#[test]
fn mailbox_and_messages() {
    let box_ = parse::parse_mailbox(&fixture("auth_frag_postbox_conversations.html")).unwrap();
    assert_eq!((box_.total, box_.unread, box_.items.len()), (Some(0), Some(0), 1));
    let c = &box_.items[0];
    assert_eq!((c.nick.as_str(), c.last_from_me, c.url.as_str()), ("otheruser", true, "https://www.parfumo.com/Postbox/R/otheruser"));
    assert_eq!(c.text, "Test message, jalon 0 bis exploration. Please ignore.");
    assert_eq!(c.last_message_at.as_deref(), Some("2026-11-12T06:25:01Z"));
    let empty = parse::parse_mailbox(&fixture("auth_frag_postbox_empty.html")).unwrap();
    assert!(empty.items.is_empty());
    assert_outdated(parse::parse_mailbox("<div>login</div>"));

    assert_eq!(parse::parse_thread_page(&fixture("auth_postbox_thread.html")).unwrap(), (1_000_002, "otheruser".to_string()));
    assert_outdated(parse::parse_thread_page(&fixture("auth_profile_own.html")));

    let messages = parse::parse_messages(&fixture("auth_frag_postbox_messages.html")).unwrap();
    assert_eq!(messages.len(), 1);
    let m = &messages[0];
    assert_eq!((m.id, m.time, m.from_me, m.author.as_str()), (100_000_001, Some(1_794_464_701), true, "testuser"));
    assert_eq!(m.text, "Test message, jalon 0 bis exploration. Please ignore.");
    assert!(parse::parse_messages("  ").unwrap().is_empty());
    assert_outdated(parse::parse_messages("<div>something else</div>"));
}

#[test]
fn notifications_empty_and_filled() {
    assert!(parse::parse_notifications(&fixture("auth_frag_notifications_empty.html")).unwrap().is_empty());
    // Shape read in the site's script (`.notification-item`, `data-id`, `data-link`): no
    // real notification was ever captured.
    let made_up = r#"<div class="notification-item" data-id="77" data-link="/Users/otheruser/Wall"><span class="pm-close">x</span><b>otheruser</b> wrote on your wall</div>"#;
    let items = parse::parse_notifications(made_up).unwrap();
    assert_eq!((items[0].id, items[0].text.as_str(), items[0].link.as_deref()), (77, "otheruser wrote on your wall", Some("/Users/otheruser/Wall")));
    assert_outdated(parse::parse_notifications("<div>login</div>"));
}

#[test]
fn followed_members() {
    let f = parse::parse_following(&fixture("auth_follow.html")).unwrap();
    assert_eq!(f.total, Some(1));
    assert_eq!(f.members.iter().map(|m| m.nick.as_str()).collect::<Vec<_>>(), ["otheruser"]);
    // The list itself, captured at jalon 4 while following one member, and its empty state.
    let list = parse::parse_following(&fixture("auth_follow_list.html")).unwrap();
    assert_eq!((list.total, list.members.len()), (Some(1), 1));
    assert_eq!((list.members[0].nick.as_str(), list.members[0].url.as_str()), ("otheruser", "https://www.parfumo.com/Users/otheruser"));
    let none = parse::parse_following(&fixture("auth_follow_list_empty.html")).unwrap();
    assert_eq!((none.total, none.members.len()), (None, 0));
    assert_outdated(parse::parse_following(&fixture("auth_profile_own.html")));
    assert_eq!(parse::parse_sub_types(r#"["follow","reviews"]"#).unwrap(), ["follow", "reviews"]);
    assert!(parse::parse_sub_types("[]").unwrap().is_empty());
    assert_outdated(parse::parse_sub_types("<html>"));
}

#[test]
fn settings_pages() {
    let n = parse::parse_settings(&fixture("auth_settings_notifications.html")).unwrap();
    assert_eq!(n.len(), 14);
    let get = |field: &str| n.iter().find(|s| s.field == field).unwrap();
    assert_eq!(
        (get("notifications").value.as_deref(), get("notifications").label.as_str(), get("notifications").section.as_deref()),
        (Some("true"), "Instant Notifications", Some("Notification"))
    );
    assert_eq!((get("notifications_messages").value.as_deref(), get("notifications_messages").kind.as_str()), (Some("false"), "toggle"));
    assert_eq!((get("hide_followers").label.as_str(), get("hide_followers").section.as_deref()), ("Hide follower lists", Some("Privacy")));

    let w = parse::parse_settings(&fixture("auth_settings_website.html")).unwrap();
    assert_eq!(w.len(), 9);
    let order = w.iter().find(|s| s.field == "forum_post_order").unwrap();
    assert_eq!(order.kind, "choice");
    assert_eq!(
        order.options.iter().map(|o| (o.value.as_str(), o.label.as_str())).collect::<Vec<_>>(),
        [("Aufsteigend", "Ascending"), ("Absteigend", "Descending")]
    );
    assert_eq!(w.iter().find(|s| s.field == "hide_souk").unwrap().section.as_deref(), Some("Souk"));
    assert_eq!(w.iter().find(|s| s.field == "quick_access").unwrap().label, "Quick access");

    let p = parse::parse_settings(&fixture("auth_personal_details.html")).unwrap();
    assert_eq!(p.iter().map(|s| s.field.as_str()).collect::<Vec<_>>(), ["gender", "country_id", "time_zone_id"]);
    assert_eq!(p[0].options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["m", "f", "d"]);
    assert!(p[1].options.len() > 200 && p[1].options.iter().any(|o| o.label == "Albania" && o.value == "6"));
    assert_eq!(p[1].label, "Country");
    assert_outdated(parse::parse_settings(&fixture("auth_profile_own.html")));
}

#[test]
fn profile_forms() {
    let prefs = parse::parse_fragrance_preferences(&fixture("auth_fragrance_preferences.html")).unwrap();
    assert_eq!((prefs.i_like.as_str(), prefs.i_dont_like.as_str()), ("", ""));
    assert_outdated(parse::parse_fragrance_preferences(&fixture("auth_profile_own.html")));
    let sig = parse::parse_signature_fragment(&fixture("auth_frag_signature_scent.html")).unwrap();
    assert_eq!((sig.id.as_str(), sig.name.as_str()), ("Dior/Sauvage_Eau_de_Toilette", "Sauvage Eau de Toilette"));
    assert!(parse::parse_signature_fragment("<div class=\"sig-sel\"></div>").is_none());
}

#[test]
fn write_forms() {
    let conf = parse::parse_wall_conf(&fixture("auth_profile_other.html")).unwrap();
    assert_eq!((conf.kind.as_str(), conf.item_id, conf.comment_id.as_str(), conf.l.as_str(), conf.h.len()), ("id_cards", 1_000_002, "0", "5", 32));
    assert_eq!((conf.p1.as_str(), conf.p2.as_str(), conf.redirect.as_str()), ("", "", ""));
    assert!(parse::parse_wall_conf(&fixture("wall_member.html")).is_none(), "an anonymous page carries no wall form");
    assert_eq!(parse::parse_forum_compose(&fixture("auth_frag_forum_compose_panel.html")).unwrap(), 18);
    assert_outdated(parse::parse_forum_compose("<div>Not allowed</div>"));
    let form = parse::parse_souk_form(&fixture("auth_frag_souk_item_panel.html")).unwrap();
    assert_eq!(
        (
            form["p"].as_str(),
            form["t"].as_str(),
            form["souk_id"].as_str(),
            form["souk_wunsch"].as_str(),
            form["zustand"].as_str(),
            form["unit"].as_str()
        ),
        ("63363", "normal_need", "0", "purchase", "any", "ml")
    );
    for field in ["tausch_gegen", "ml_inhalt", "preisvorstellung", "versand", "notes"] {
        assert_eq!(form[field], "", "{field}");
    }
    assert!(parse::parse_souk_form(&fixture("auth_frag_souk_offer_unavailable.html")).is_none(), "offers are refused to a recent account");
}
