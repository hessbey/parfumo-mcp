//! Parser tests on HTML recorded from parfumo.com (public pages, 2026-11-11). No network.

use parfumo_client::{ClientError, parse, site::markers};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn assert_outdated<T: std::fmt::Debug>(result: Result<T, ClientError>) {
    match result {
        Err(ClientError::ParserOutdated(msg)) => assert!(!msg.is_empty()),
        other => panic!("expected ParserOutdated, got {other:?}"),
    }
}

const SAUVAGE_URL: &str = "https://www.parfumo.com/Perfumes/Dior/Sauvage_Eau_de_Toilette";

#[test]
fn perfume_page_flat_notes() {
    let (p, ctx) = parse::parse_perfume_page(&fixture("perfume_sauvage.html")).unwrap();
    assert_eq!(p.id, "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(p.url, SAUVAGE_URL);
    assert_eq!(p.p_id, Some(63363));
    assert_eq!(p.name, "Sauvage");
    assert_eq!(p.brand.id, "Dior");
    assert_eq!(p.brand.name, "Dior");
    assert_eq!(p.brand.url, "https://www.parfumo.com/Perfumes/Dior");
    assert_eq!(p.year, Some(2015));
    assert_eq!(p.gender.as_deref(), Some("men"));
    assert_eq!(p.concentration.as_deref(), Some("Eau de Toilette"));
    assert_eq!(p.perfumers.len(), 1);
    assert_eq!(p.perfumers[0].name, "François Demachy");
    assert_eq!(p.perfumers[0].id, "Franois_Demachy");
    assert!(p.notes.top.is_empty() && p.notes.heart.is_empty() && p.notes.base.is_empty());
    assert_eq!(p.notes.flat.len(), 9);
    assert_eq!(p.notes.flat[0], "Ambrox");
    assert_eq!(p.notes.flat[8], "Vetiver");
    assert_eq!(p.accords, ["Fresh", "Spicy", "Synthetic", "Citrus", "Woody"]);
    let dims: Vec<(&str, f32, Option<u32>)> = p.ratings.iter().map(|r| (r.dimension.as_str(), r.average, r.votes)).collect();
    assert_eq!(
        dims,
        [
            ("scent", 7.5, Some(6803)),
            ("durability", 7.8, Some(6392)),
            ("sillage", 7.7, Some(6368)),
            ("bottle", 7.7, Some(6424)),
            ("pricing", 7.4, Some(4820)),
        ]
    );
    assert_eq!(p.ranking.as_deref(), Some("Ranked 30 in Men's Perfume"));
    assert_eq!(
        p.description.as_deref(),
        Some("A perfume by Dior for men, released in 2015. The scent is fresh-spicy. It is being marketed by LVMH.")
    );
    assert!(p.facts.as_deref().unwrap().contains("Fragrance Foundation"));
    assert_eq!(p.reviews_count, Some(378));
    assert_eq!(p.statements_count, Some(1322));
    assert!(p.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/"));
    assert!(p.warnings.is_empty());

    assert_eq!(ctx.p_id, 63363);
    assert_eq!(ctx.h, "0d03f07b7a18e56da70ee2ef14bba111");
    assert_eq!(ctx.class_h.as_deref(), Some("928478793683e3b88ad871312cb1832f"));
    assert_eq!(ctx.reviews_total, Some(378));
    assert_eq!(ctx.statements_total, Some(1322));
}

#[test]
fn perfume_page_pyramid() {
    let (p, ctx) = parse::parse_perfume_page(&fixture("perfume_pyramid.html")).unwrap();
    assert_eq!(p.id, "Dior/Dior_Homme_Intense_2011");
    assert_eq!(p.name, "Dior Homme Intense");
    assert_eq!(p.year, Some(2011));
    assert_eq!(p.concentration, None);
    assert_eq!(p.notes.top, ["Florentine Iris absolute"]);
    assert_eq!(p.notes.heart, ["Ecuadorian ambrette seed"]);
    assert_eq!(p.notes.base, ["Virginia cedar"]);
    assert!(p.notes.flat.is_empty());
    assert_eq!(ctx.p_id, 13889);
    assert_eq!(ctx.reviews_total, Some(522));
}

#[test]
fn perfume_page_without_votes() {
    let (p, ctx) = parse::parse_perfume_page(&fixture("perfume_unrated.html")).unwrap();
    assert_eq!(p.id, "ayaam-perfumes/marmo-sunset");
    assert_eq!(p.gender.as_deref(), Some("unisex"));
    assert!(p.ratings.is_empty());
    assert_eq!(p.notes.top, ["Jasmine", "Vanilla orchid"]);
    // No rating block on this page: the tokens come from the inline scripts.
    assert_eq!(ctx.p_id, 401140);
    assert_eq!(ctx.h.len(), 32);
    assert_eq!(ctx.reviews_total, Some(0));
    assert_eq!(ctx.statements_total, None);
}

#[test]
fn reviews_embedded_in_perfume_page() {
    let reviews = parse::parse_reviews(&fixture("perfume_sauvage.html"), SAUVAGE_URL).unwrap();
    assert_eq!(reviews.len(), 5);
    let r = &reviews[0];
    assert_eq!(r.id, 77985);
    assert_eq!(r.author, "member030");
    assert_eq!(r.author_url.as_deref(), Some("https://www.parfumo.com/Users/member030"));
    assert_eq!(r.date.as_deref(), Some("2015-08-27"));
    assert_eq!(r.title.as_deref(), Some("Clockwork Orange"));
    assert!(r.text.starts_with("Powdered orange juice concentrate"));
    assert!(r.text.contains("\n\nSauvage follows in the vein"), "paragraph breaks are kept");
    assert!(!r.text.contains('<'));
    assert_eq!(r.awards, Some(12));
    assert_eq!(r.url, format!("{SAUVAGE_URL}/reviews/77985"));
}

#[test]
fn reviews_fragment_full_and_excerpt() {
    let full = parse::parse_reviews(&fixture("frag_reviews_full.html"), SAUVAGE_URL).unwrap();
    assert_eq!(full.len(), 3);
    assert_eq!(full[0].id, 721243);
    assert_eq!(full[0].author, "member315");
    assert_eq!(full[0].title.as_deref(), Some("Pepper, Citrus, Sun, and Woods"));
    assert!(!full[0].truncated);
    assert!(full[0].text.ends_with("I do reach for it every now and then."));
    assert_eq!(full[0].ratings.scent, Some(7.0));
    assert_eq!(full[0].ratings.durability, Some(7.0));
    assert_eq!(full[0].ratings.sillage, Some(7.0));
    assert_eq!(full[0].ratings.bottle, None);
    assert_eq!(full[0].url, format!("{SAUVAGE_URL}/reviews/721243"));
    assert_eq!(full[0].comments, Some(0));

    let excerpt = parse::parse_reviews(&fixture("frag_reviews_excerpt.html"), SAUVAGE_URL).unwrap();
    assert_eq!(excerpt.len(), 10);
    assert!(excerpt[0].truncated, "excerpt fragments are flagged");
    assert_eq!(excerpt[1].ratings.scent, Some(7.5));
    assert_eq!(excerpt[1].ratings.durability, Some(9.0));
}

#[test]
fn statements_fragment() {
    let items = parse::parse_statements(&fixture("frag_statements.html"), SAUVAGE_URL).unwrap();
    assert_eq!(items.len(), 10);
    let s = &items[0];
    assert_eq!(s.id, 2800120);
    assert_eq!(s.author, "member290");
    assert_eq!(s.author_url.as_deref(), Some("https://www.parfumo.com/Users/member290"));
    assert_eq!(s.url, format!("{SAUVAGE_URL}/statements/2800120"));
    assert_eq!(s.text, "Me desfiz do meu. Cheiro de sabão em pó que nem performar performa.");
    assert_eq!(s.ratings.scent, Some(5.5));
    assert_eq!(s.ratings.durability, Some(6.0));
    assert_eq!(s.ratings.sillage, Some(7.0));
    assert!(s.date.is_some());
    // The perfume page embeds the first statements too.
    assert_eq!(parse::parse_statements(&fixture("perfume_sauvage.html"), SAUVAGE_URL).unwrap().len(), 5);
}

#[test]
fn similar_fragment_and_empty_state() {
    let items = parse::parse_similar(&fixture("frag_similar_all.html")).unwrap();
    assert_eq!(items.len(), 30);
    let first = &items[0];
    assert_eq!(first.p_id, 102281);
    assert_eq!(first.name, "Sauvage");
    assert_eq!(first.brand.as_deref(), Some("Dior"));
    assert_eq!(first.concentration.as_deref(), Some("Eau de Parfum"));
    assert_eq!(first.percent, Some(91.4));
    assert_eq!(first.url, None, "the site gives no link in this list");
    assert_eq!(items[3].name, "Writer");
    assert_eq!(items[3].brand.as_deref(), Some("Cyrus"));

    assert!(parse::parse_similar(&fixture("frag_inspiration_empty.html")).unwrap().is_empty());
    assert!(parse::parse_similar("").unwrap().is_empty());
    assert_outdated(parse::parse_similar("<div class=\"something_else\">changed</div>"));
}

#[test]
fn similar_sneak_peek() {
    let (url, votes) = parse::parse_sneak_peek(&fixture("frag_sneak_peek.html")).unwrap();
    assert_eq!(url, "https://www.parfumo.com/Perfumes/Dior/Sauvage_Eau_de_Parfum");
    assert_eq!(votes, Some(268));
    assert_eq!(parse::perfume_id_from_url(&url).as_deref(), Some("Dior/Sauvage_Eau_de_Parfum"));
    assert_outdated(parse::parse_sneak_peek("<div class=\"header\"></div>"));
}

#[test]
fn classification_chart() {
    let groups = parse::parse_classification(&fixture("frag_classification_pie.html")).unwrap();
    let names: Vec<&str> = groups.iter().map(|g| g.group.as_str()).collect();
    assert_eq!(names, ["Type", "Occasion", "Season", "Audience & Style"]);
    assert_eq!(groups[0].votes, Some(4710));
    assert_eq!(groups[0].shares[0].label, "Fresh");
    assert_eq!(groups[0].shares[0].percent, 25.0);
    let season: Vec<(&str, f32)> = groups[2].shares.iter().map(|s| (s.label.as_str(), s.percent)).collect();
    assert_eq!(season, [("Spring", 30.0), ("Summer", 27.0), ("Fall", 26.0), ("Winter", 17.0)]);
    assert_eq!(groups[1].shares.iter().find(|s| s.label == "Night Out").map(|s| s.percent), Some(15.0));

    assert!(parse::parse_classification(&fixture("frag_classification_empty.html")).unwrap().is_empty());
    assert_outdated(parse::parse_classification("<div id=\"classification_community\"><p>new layout</p></div>"));
}

#[test]
fn dupes_directory() {
    let groups = parse::parse_dupes(&fixture("frag_dupes.html")).unwrap();
    assert_eq!(groups.len(), 21);
    let g = &groups[0];
    assert_eq!(g.original.id, "Dior/Eau_Sauvage_Extrme_1982_Eau_de_Toilette_Concentree");
    assert_eq!(g.original.name, "Eau Sauvage Extrême (1982)");
    assert_eq!(g.original.concentration.as_deref(), Some("Eau de Toilette Concentrée"));
    assert_eq!(g.original.brand.as_deref(), Some("Dior"));
    assert_eq!(g.dupes.len(), 3);
    assert_eq!(g.dupes[0].perfume.id, "Lorenzo_Villoresi/aura-maris");
    assert_eq!(g.dupes[0].perfume.name, "Aura Maris");
    assert_eq!(g.dupes[0].perfume.brand.as_deref(), Some("Lorenzo Villoresi"));
    assert_eq!(g.dupes[0].votes, Some(7));
    assert_eq!(g.dupes[2].perfume.name, "Terre d'Hermès");
    assert_eq!(g.dupes[2].perfume.concentration.as_deref(), Some("Eau de Toilette"));
    assert!(groups.iter().any(|g| g.original.id == "Dior/Sauvage_Eau_de_Toilette"));
    // "Nothing found" answer of the directory.
    assert!(parse::parse_dupes("<div class=\"text-center\"><img class=\"nothing_info_img\"></div>").unwrap().is_empty());
}

#[test]
fn search_results_and_pagination() {
    let html = fixture("search.html");
    let page = parse::parse_listing(&html, 1).unwrap();
    assert_eq!(page.items.len(), 20);
    assert_eq!(page.page, 1);
    assert_eq!(page.total_items, Some(151));
    assert_eq!(page.total_pages, Some(8));
    let first = &page.items[0];
    assert_eq!(first.id, "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(first.name, "Sauvage");
    assert_eq!(first.concentration.as_deref(), Some("Eau de Toilette"));
    assert_eq!(first.brand.as_deref(), Some("Dior"));
    assert_eq!(first.year, Some(2015));
    assert_eq!(first.gender.as_deref(), Some("men"));
    assert_eq!(first.url, SAUVAGE_URL);
    assert_eq!(page.items[1].id, "Dior/sauvage-elixir");
    assert_eq!(page.items[1].concentration, None);

    // Far out-of-range pages are answered with page 1 by the site: detected from "1 - 20 by 151".
    let clamped = parse::parse_listing(&html, 99).unwrap();
    assert!(clamped.items.is_empty());
    assert_eq!(clamped.total_items, Some(151));
}

#[test]
fn search_without_result_and_past_the_end() {
    let empty = parse::parse_listing(&fixture("search_empty.html"), 1).unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.total_items, Some(0));

    let past = parse::parse_listing(&fixture("search_past_end.html"), 9).unwrap();
    assert!(past.items.is_empty());
    assert_eq!(past.total_items, Some(151));
    assert_eq!(past.total_pages, Some(8));
}

#[test]
fn brand_page() {
    let b = parse::parse_brand_page(&fixture("brand.html"), 1).unwrap();
    assert_eq!(b.brand.id, "Dior");
    assert_eq!(b.brand.name, "Dior");
    assert_eq!(b.brand.numeric_id, Some(28));
    assert_eq!(b.brand.url, "https://www.parfumo.com/Perfumes/Dior");
    assert_eq!(b.country.as_deref(), Some("France"));
    assert_eq!(b.founded, Some(1946));
    assert_eq!(b.perfumes.items.len(), 20);
    assert_eq!(b.perfumes.total_items, Some(344));
    assert_eq!(b.perfumes.total_pages, Some(18));
    let first = &b.perfumes.items[0];
    assert_eq!(first.id, "Dior/Dior_Homme_Intense_2011");
    assert_eq!(first.name, "Dior Homme Intense (2011)");
    assert_eq!(first.brand.as_deref(), Some("Dior"));
    assert_eq!(first.year, Some(2011));
    assert_eq!(first.rating, Some(8.5));
    assert_eq!(first.rating_count, Some(9084));
    assert_eq!(first.reviews_count, Some(523));
    assert_eq!(first.gender.as_deref(), Some("men"));
}

#[test]
fn brand_page_two_has_a_linked_heading() {
    let b = parse::parse_brand_page(&fixture("brand_page2.html"), 2).unwrap();
    assert_eq!(b.brand.name, "Dior");
    assert_eq!(b.brand.url, "https://www.parfumo.com/Perfumes/Dior");
    assert_eq!(b.perfumes.page, 2);
    assert_eq!(b.perfumes.items.len(), 20);
    assert_eq!(b.perfumes.total_items, Some(344));
}

#[test]
fn perfumer_page() {
    let p = parse::parse_perfumer_page(&fixture("perfumer.html"), 1).unwrap();
    assert_eq!(p.perfumer.id, "Franois_Demachy");
    assert_eq!(p.perfumer.name, "François Demachy");
    assert_eq!(p.perfumer.numeric_id, Some(131));
    assert_eq!(p.country.as_deref(), Some("France"));
    assert_eq!(p.perfumes.items.len(), 20);
    assert_eq!(p.perfumes.total_items, Some(188));
    assert_eq!(p.perfumes.total_pages, Some(10));
}

#[test]
fn note_and_note_category_pages() {
    let n = parse::parse_note_page(&fixture("note.html"), 1).unwrap();
    assert_eq!(n.note.id, "Bergamot");
    assert_eq!(n.note.name, "Bergamot");
    assert_eq!(n.note.numeric_id, Some(311));
    assert_eq!(n.note.url, "https://www.parfumo.com/Fragrance_Note/Bergamot");
    assert!(!n.is_category);
    let cats: Vec<&str> = n.categories.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(cats, ["Bergamot", "Citric_notes"]);
    assert_eq!(n.perfumes.items.len(), 20);
    assert_eq!(n.perfumes.total_items, Some(43518));
    assert_eq!(n.perfumes.total_pages, Some(50), "the site lists at most 1000 results");

    let c = parse::parse_note_page(&fixture("note_category.html"), 1).unwrap();
    assert!(c.is_category);
    assert_eq!(c.note.id, "Bergamot");
    assert_eq!(c.note.url, "https://www.parfumo.com/Fragrance_Notes/Bergamot");
    assert!(c.categories.iter().any(|x| x.id == "Citric_notes"));
    assert_eq!(c.perfumes.items.len(), 20);
}

#[test]
fn top_list() {
    let html = fixture("top_men.html");
    let top = parse::parse_listing(&html, 1).unwrap();
    assert_eq!(top.items.len(), 100);
    assert_eq!(top.total_pages, Some(5));
    let first = &top.items[0];
    assert_eq!(first.rank, Some(1));
    assert_eq!(first.id, "Louis_Vuitton/imagination");
    assert_eq!(first.name, "Imagination");
    assert_eq!(first.brand.as_deref(), Some("Louis Vuitton"));
    assert_eq!(first.rating, Some(8.9));
    assert_eq!(first.rating_count, Some(10161));
    assert_eq!(first.reviews_count, Some(711));
    assert_eq!(top.items[99].rank, Some(100));
    assert!(parse::parse_listing(&html, 6).unwrap().items.is_empty());
}

#[test]
fn trends_sections() {
    let sections = parse::parse_trends(&fixture("trends.html")).unwrap();
    let titles: Vec<&str> = sections.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(titles, ["Most added to collections", "Most wished", "Most worn"]);
    for (_, items) in &sections {
        assert_eq!(items.len(), 20);
        assert_eq!(items[0].rank, Some(1));
        assert!(items[0].trend_count.is_some());
    }
    assert_eq!(sections[0].1[0].id, "french-avenue/liquid-brun");
    assert_eq!(sections[0].1[0].trend_count, Some(2711));
    assert_eq!(sections[0].1[0].brand.as_deref(), Some("French Avenue"));
}

#[test]
fn autocompletion_fragments() {
    let brands = parse::parse_label_filter(&fixture("frag_label_brands.html"), "data-b_id");
    assert_eq!(brands, [(28, "Dior".to_string()), (20479, "Diorama Perfumes".to_string())]);
    let notes = parse::parse_label_filter(&fixture("frag_label_notes.html"), "data-n_inc_id");
    assert_eq!(notes[0], (311, "Bergamot".to_string()));
    assert_eq!(notes[1], (3804, "Italian bergamot".to_string()));
    let links = parse::parse_livesearch_links(&fixture("frag_livesearch_brand.html"));
    assert_eq!(links[0], ("Dior".to_string(), "https://www.parfumo.com/Perfumes/Dior".to_string()));
    let note_slugs = parse::parse_livesearch_notes(&fixture("frag_livesearch_note.html"));
    assert_eq!(note_slugs[0], ("Bergamot".to_string(), "Bergamot".to_string()));
    assert_eq!(note_slugs[2], ("Calabrian bergamot".to_string(), "Calabrian_bergamot".to_string()));
}

#[test]
fn not_found_page_is_not_found() {
    let html = fixture("notfound.html");
    assert!(parse::is_not_found_page(&html));
    assert!(matches!(parse::parse_perfume_page(&html), Err(ClientError::NotFound(_))));
    assert!(matches!(parse::parse_listing(&html, 1), Err(ClientError::NotFound(_))));
    assert!(matches!(parse::parse_brand_page(&html, 1), Err(ClientError::NotFound(_))));
    assert!(!parse::is_not_found_page(&fixture("perfume_sauvage.html")));
}

#[test]
fn changed_structure_is_parser_outdated_never_a_panic() {
    let perfume = fixture("perfume_sauvage.html");
    assert_outdated(parse::parse_perfume_page(&perfume.replace("p_name_h1", "p_title_v2")));
    assert_outdated(parse::parse_perfume_page(&perfume.replace("rel=\"canonical\"", "rel=\"other\"")));
    // Tokens gone: the record could be read but no internal call would work.
    let no_tokens = perfume.replace("data-h=", "data-x=").replace("getPerfume", "loadPerfume");
    assert_outdated(parse::parse_perfume_page(&no_tokens));
    // Truncated documents at arbitrary byte offsets (kept on char boundaries).
    for cut in [0usize, 500, 20_000, 40_000, 60_000] {
        let mut end = cut.min(perfume.len());
        while !perfume.is_char_boundary(end) {
            end -= 1;
        }
        let _ = parse::parse_perfume_page(&perfume[..end]);
        let _ = parse::parse_listing(&perfume[..end], 1);
        let _ = parse::parse_reviews(&perfume[..end], SAUVAGE_URL);
    }
    assert_outdated(parse::parse_perfume_page("<html><head><title>x</title></head><body></body></html>"));

    let search = fixture("search.html");
    assert_outdated(parse::parse_listing(&search.replace("pgrid", "newgrid"), 1));
    assert_outdated(parse::parse_brand_page(&fixture("brand.html").replace("<h1", "<h5").replace("</h1>", "</h5>"), 1));
    assert_outdated(parse::parse_reviews("<div class=\"review-v2\">text</div>", SAUVAGE_URL));
    assert_outdated(parse::parse_statements("<div class=\"statement-v2\">text</div>", SAUVAGE_URL));
    assert_outdated(parse::parse_trends(&search));
}

#[test]
fn alive_json_served_as_html() {
    assert_eq!(parse::parse_alive(&fixture("alive.json")).unwrap(), "fixturecsrfkey00");
    assert_outdated(parse::parse_alive(""));
    assert_outdated(parse::parse_alive("{\"error\":\"Authentication required\"}"));
}

#[test]
fn challenge_and_login_markers() {
    // Normal pages embed Cloudflare's passive beacon: that is not a challenge.
    assert!(!parse::is_challenge(&fixture("perfume_sauvage.html")));
    assert!(!parse::is_challenge(&fixture("login_error.html")));
    assert!(parse::is_challenge("<html><head><title>Just a moment...</title></head></html>"));
    assert!(parse::is_challenge("<div class=\"cf-turnstile\" data-sitekey=\"x\"></div>"));
    assert!(fixture("login_error.html").contains(markers::INVALID_LOGIN));
}

#[test]
fn page_text_is_clean() {
    let page = parse::parse_page_text(&fixture("brand.html"), "https://www.parfumo.com/Perfumes/Dior", 50).unwrap();
    assert_eq!(page.title.as_deref(), Some("Dior » Fragrances, Reviews and Information"));
    assert!(page.text.contains("Dior Homme Intense (2011)"));
    assert!(!page.text.contains('<') && !page.text.contains("function("));
    assert!(!page.text.contains("Forgot your Password"), "login modal is not page content");
    assert!(page.links.len() <= 50 && !page.links.is_empty());
    assert!(page.links.iter().all(|l| l.url.starts_with("https://www.parfumo.com/")));
    assert!(page.links.iter().any(|l| l.url == "https://www.parfumo.com/Perfumes/Dior/Dior_Homme_Intense_2011"));
    assert!(matches!(parse::parse_page_text("  ", "https://www.parfumo.com/x", 5), Err(ClientError::Upstream(_))));
}

#[test]
fn fixtures_contain_no_session_secret() {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let key = regex::Regex::new(r#"csrf_key['"]?\s*[:=]\s*['"]([A-Za-z0-9]{16})['"]"#).unwrap();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for cap in key.captures_iter(&text) {
            assert_eq!(&cap[1], "fixturecsrfkey00", "{} holds a real csrf_key", path.display());
        }
        assert!(!text.contains("PHPSESSID="), "{} holds a session cookie", path.display());
    }
}

// --- jalon 0 bis: logged-in fixtures (`auth_*`), captured on the test account and scrubbed ---

/// Placeholder every 32-hex token (csrf_key, `h`, push channel) is replaced with.
const AUTH_HEX: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn auth_fixtures_are_scrubbed() {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let hex = regex::Regex::new(r"[0-9a-f]{32,}").unwrap();
    let email = regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[a-z]{2,}").unwrap();
    let cfemail = regex::Regex::new(r#"data-cfemail="[^"]+""#).unwrap();
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.file_name().unwrap().to_string_lossy().starts_with("auth_") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        for m in hex.find_iter(&text) {
            assert!(AUTH_HEX.repeat(4).starts_with(m.as_str()), "{} holds a real token", path.display());
        }
        assert!(!email.is_match(&text), "{} holds an e-mail address", path.display());
        assert!(!text.contains("pf_auth="), "{} holds a login cookie", path.display());
        assert!(!cfemail.is_match(&text), "{} holds a protected e-mail", path.display());
    }
    assert!(seen >= 40, "auth fixtures missing");
}

/// ShelfControl embeds the whole list as one JSON object in the page: the richest
/// source for a collection (DISCOVERY §5).
#[test]
fn auth_shelfcontrol_embeds_json_rows() {
    let html = fixture("auth_shelfcontrol.html");
    let start = html.find(r#"{"rows":["#).expect("rows JSON");
    let end = start + html[start..].find("</script>").expect("end of JSON script");
    let data: serde_json::Value = serde_json::from_str(html[start..end].trim()).expect("valid JSON");
    let row = &data["rows"][0];
    assert_eq!(row["p"], 63363);
    assert_eq!(row["title"], "Sauvage (Eau de Toilette)");
    assert_eq!(row["bottle"], "100");
    assert_eq!(row["ml"], 60);
    assert_eq!(row["paid"], 85);
    assert_eq!(row["cur"], "EUR");
    assert_eq!(row["memo"], "test note");
    assert_eq!(row["ratings"].as_array().unwrap().len(), 5);
    assert_eq!(data["brands"]["28"][0], "Dior");
    assert!(html.contains(r#"<meta name="csrf-token" content="0123456789abcdef0123456789abcdef""#));

    let empty = fixture("auth_shelfcontrol_empty.html");
    let start = empty.find(r#"{"rows":["#).expect("rows JSON");
    assert!(empty[start..].starts_with(r#"{"rows":[]"#));
}

#[test]
fn auth_api_answers_have_the_documented_shapes() {
    let json = |name: &str| serde_json::from_str::<serde_json::Value>(&fixture(name)).unwrap();
    assert_eq!(json("auth_api_wardrobe_update_details.json")["status"], "ok");
    assert_eq!(json("auth_api_memo_create.json")["text"], "test note");
    assert!(json("auth_api_wardrobe_add_list.json")["id"].as_u64().unwrap() > 0);
    assert_eq!(json("auth_api_shelfcontrol_columns.json")["ok"], true);
    assert_eq!(json("auth_api_shelfcontrol_views.json")["views"][0]["name"], "TestView");
    assert_eq!(json("auth_api_shelfcontrol_bulk.json")["lists"].as_array().unwrap().len(), 8);
    assert_eq!(json("auth_api_perfume_add_vote.json")["status"], "ok");
    assert_eq!(json("auth_api_error_invalid_csrf.json")["error"], "Invalid CSRF token");
    assert_eq!(json("auth_api_error_auth_required.json")["error"], "Authentication required");

    let csv = fixture("auth_wardrobe_export.csv");
    let mut lines = csv.trim_start_matches('\u{feff}').lines();
    assert_eq!(lines.next().unwrap(), r#""Perfume","Brand","Year","Gender","Quantity","Bottle Size","Paid","Date of Entry","Notes","Notebook""#);
    assert!(lines.next().unwrap().starts_with(r#""Sauvage (Eau de Toilette)","Dior","2015","Men","1""#));
}

// --- jalon 2: collection and wear parsers on the logged-in fixtures ---

#[test]
fn shelfcontrol_page_gives_lists_columns_and_rows() {
    let shelf = parse::parse_shelf(&fixture("auth_shelfcontrol.html")).unwrap();
    assert_eq!((shelf.list.id, shelf.list.slug.as_str(), shelf.list.name.as_str()), (1, "have", "I have"));
    assert_eq!(shelf.lists.len(), 8);
    assert_eq!(shelf.lists.iter().map(|l| l.slug.as_str()).take(7).collect::<Vec<_>>(), ["have", "had", "want", "memo", "tested", "filling", "mini"]);
    let custom = shelf.lists.last().unwrap();
    assert!(custom.custom && custom.id > 7 && custom.name == "TestList" && !custom.hidden);
    assert!(shelf.lists.iter().take(7).all(|l| !l.custom));
    assert_eq!(
        shelf.columns,
        vec![parfumo_client::CustomColumn { id: 178, name: "TestCol".into() }],
        "country/community headers are not custom columns"
    );

    assert_eq!(shelf.items.len(), 1);
    let i = &shelf.items[0];
    assert_eq!((i.entry_id, i.p_id, i.perfume_id.as_str()), (119519310, 63363, "Dior/Sauvage_Eau_de_Toilette"));
    assert_eq!(i.name, "Sauvage (Eau de Toilette)", "no HTML in the name");
    assert_eq!(
        (i.brand.as_deref(), i.year, i.gender.as_deref(), i.concentration.as_deref()),
        (Some("Dior"), Some(2015), Some("men"), Some("Eau de Toilette"))
    );
    assert_eq!(
        (i.quantity, i.bottle_size.as_deref(), i.remaining_ml, i.price_paid, i.currency.as_deref()),
        (Some(1), Some("100"), Some(60.0), Some(85.0), Some("EUR"))
    );
    assert_eq!(i.private_note.as_deref(), Some("test note"));
    assert_eq!(i.added.as_deref(), Some("2026-11-12"));
    assert_eq!((i.wear_count, i.last_worn.as_deref()), (0, None));
    assert!(i.my_ratings.is_empty());
    assert_eq!((i.community_rating, i.community_votes), (Some(7.5), Some(6805)));
    assert_eq!(i.perfumers, ["François Demachy"]);
    assert_eq!(i.accords, ["Fresh", "Spicy", "Synthetic"]);
    assert_eq!(i.notes.len(), 9);
    assert_eq!((i.seasons.clone(), i.occasions.clone()), (vec!["Spring".to_string(), "Summer".into()], vec!["Daily".to_string(), "Leisure".into()]));
    assert_eq!(i.country.as_deref(), Some("France"));

    // An empty list serialises its dictionaries as `[]`.
    let empty = parse::parse_shelf(&fixture("auth_shelfcontrol_empty.html")).unwrap();
    assert!(empty.items.is_empty() && empty.lists.len() >= 7);
    // Any other page is a structure change, not an empty collection.
    assert_outdated(parse::parse_shelf(&fixture("auth_tracker.html")));
    assert_outdated(parse::parse_shelf(&fixture("auth_shelfcontrol.html").replace("psheet-data", "psheet-other")));
}

#[test]
fn list_references_resolve_by_slug_alias_id_and_name() {
    let shelf = parse::parse_shelf(&fixture("auth_shelfcontrol.html")).unwrap();
    let id = |r: &str| parfumo_client::find_list(&shelf.lists, r).map(|l| l.id);
    assert_eq!(id("have"), Some(1));
    assert_eq!(id("Wish List"), Some(3));
    assert_eq!(id("wishlist"), Some(3));
    assert_eq!(id(" WATCH-LIST "), Some(4));
    assert_eq!(id("decants"), Some(6));
    assert_eq!(id("Miniatures"), Some(7));
    assert_eq!(id("7"), Some(7));
    assert_eq!(id("testlist"), Some(922018));
    assert_eq!(id("922018"), Some(922018));
    assert_eq!(id("nothing"), None);
    assert_eq!(id("12345"), None);
}

#[test]
fn wear_tracker_and_calendar_rows() {
    let wears = parse::parse_wear_tracker(&fixture("auth_tracker.html")).unwrap();
    assert_eq!(wears.len(), 1);
    let w = &wears[0];
    assert_eq!(
        (w.wear_id, w.perfume_id.as_str(), w.name.as_str(), w.brand.as_deref()),
        (22205264, "Dior/Sauvage_Eau_de_Toilette", "Sauvage (Eau de Toilette)", Some("Dior"))
    );
    assert_eq!(w.worn_at.as_deref(), Some("2026-11-12T07:21"));
    assert_eq!(w.comment, None);

    let month = parse::parse_wear_calendar(&fixture("auth_frag_calendar_month.html")).unwrap();
    assert_eq!(month.len(), 1);
    assert_eq!((month[0].wear_id, month[0].worn_at.as_deref(), month[0].comment.as_deref()), (22205264, Some("2026-11-11T08:00"), Some("test note")));

    assert_eq!(parse::parse_wear_when("Monday, 01/02/2026, 12:05 AM").as_deref(), Some("2026-02-01T00:05"));
    assert_eq!(parse::parse_wear_when("Monday, 01/02/2026, 12:05 PM").as_deref(), Some("2026-02-01T12:05"));
    assert_eq!(parse::parse_wear_when("Monday, 01/02/2026, 11:59 PM").as_deref(), Some("2026-02-01T23:59"));
    assert_eq!(parse::parse_wear_when("A moment ago"), None);
    assert_eq!(parse::iso_date(1794464318), "2026-11-12");
    assert_eq!(parse::iso_date(0), "1970-01-01");
    assert_eq!(parse::civil_from_days(19_782), (2024, 2, 29));

    // Other fragments are refused rather than read as "no wear".
    assert_outdated(parse::parse_wear_calendar(&fixture("auth_frag_i_wear.html")));
    assert_outdated(parse::parse_wear_tracker(&fixture("notfound.html")));
}

// --- jalon 3: ratings, classification, reviews and statements ---

#[test]
fn rate_panel_gives_the_members_ratings_out_of_ten() {
    let r = parse::parse_rate_panel(&fixture("auth_frag_rate_panel.html")).unwrap();
    assert_eq!(r.scent, Some(7.5), "3.75 stars out of 5");
    assert_eq!((r.durability, r.sillage, r.bottle, r.pricing), (None, None, None, None), "a hidden remove link means not rated");
    assert_outdated(parse::parse_rate_panel("<div>nothing</div>"));
}

#[test]
fn classify_panel_lists_every_label_and_the_members_votes() {
    let choices = parse::parse_classify_panel(&fixture("auth_frag_classify_panel.html")).unwrap();
    assert_eq!(choices.len(), 21 + 4 + 4 + 6);
    let groups: Vec<&str> = choices.iter().map(|c| c.group.as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    assert_eq!(groups, ["Occasion", "Season", "Style", "Type"]);
    let voted: Vec<&str> = choices.iter().filter(|c| c.voted).map(|c| c.label.as_str()).collect();
    assert_eq!(voted, ["Fresh"]);
    let night = choices.iter().find(|c| c.label == "Night Out").unwrap();
    assert_eq!((night.group.as_str(), night.group_id, night.label_id), ("Occasion", 2, 7));
    assert_outdated(parse::parse_classify_panel("<div>nothing</div>"));
}

#[test]
fn own_review_and_statement_reads() {
    use parfumo_client::TextKind;
    let review = parse::parse_own_draft(&fixture("auth_api_get_user_review.json"), TextKind::Review).unwrap();
    assert_eq!((review.title, review.text.as_str()), (None, ""));
    let statement = parse::parse_own_draft(&fixture("auth_api_get_user_statement.json"), TextKind::Statement).unwrap();
    assert_eq!(statement.text, "");
    let written = parse::parse_own_draft(r#"{"review_title":"Title","review_text":" Body "}"#, TextKind::Review).unwrap();
    assert_eq!((written.title.as_deref(), written.text.as_str()), (Some("Title"), "Body"));
    assert_outdated(parse::parse_own_draft("{\"error\":\"x\"}", TextKind::Statement));
    assert_outdated(parse::parse_own_draft("", TextKind::Review));
}

#[test]
fn forms_without_a_text_have_no_delete_button() {
    use parfumo_client::TextKind;
    assert_eq!(parse::parse_delete_handle(&fixture("auth_frag_review_panel.html"), TextKind::Review).unwrap(), None);
    assert_eq!(parse::parse_delete_handle(&fixture("auth_frag_statement_panel.html"), TextKind::Statement).unwrap(), None);
    // Shape of the button as the form's own script reads it (data-id, data-p, data-h); the
    // button itself was never served to the test account, which wrote nothing.
    let with_button = fixture("auth_frag_statement_panel.html").replace(
        "</form>",
        &format!("<button class=\"action_delete_statement pbtn\" data-id=\"42\" data-p=\"63363\" data-h=\"{AUTH_HEX}\">Delete</button></form>"),
    );
    let handle = parse::parse_delete_handle(&with_button, TextKind::Statement).unwrap().unwrap();
    assert_eq!((handle.id, handle.p_id, handle.h.as_str()), (42, 63363, AUTH_HEX));
    assert_outdated(parse::parse_delete_handle(&with_button.replace(AUTH_HEX, "nope"), TextKind::Statement));
    assert_outdated(parse::parse_delete_handle("<div>nothing</div>", TextKind::Review));
}

#[test]
fn my_ratings_page_lists_rated_perfumes() {
    let page = parse::parse_my_ratings(&fixture("auth_ratings.html"), 1).unwrap();
    assert_eq!((page.items.len(), page.total_pages), (1, Some(1)));
    let r = &page.items[0];
    assert_eq!((r.perfume_id.as_str(), r.p_id, r.rating), ("Dior/Sauvage_Eau_de_Toilette", 63363, 7.5));
    assert_eq!((r.name.as_str(), r.brand.as_deref(), r.rated.as_deref()), ("Sauvage (Eau de Toilette)", Some("Dior"), Some("Just now")));
    let empty = parse::parse_my_ratings(&fixture("auth_ratings_empty.html"), 1).unwrap();
    assert!(empty.items.is_empty());
    assert_outdated(parse::parse_my_ratings(&fixture("auth_tracker.html"), 1));
}

#[test]
fn member_reviews_and_statements_pages() {
    let reviews = parse::parse_user_reviews(&fixture("user_reviews.html"), 1).unwrap();
    assert_eq!((reviews.items.len(), reviews.total_items, reviews.total_pages), (5, Some(20), Some(4)));
    let r = &reviews.items[0];
    assert_eq!((r.id, r.perfume_id.as_str(), r.perfume_name.as_str()), (724459, "D_S__Durga/steamed-rainbow", "Steamed Rainbow"));
    assert_eq!(r.url, "https://www.parfumo.com/Perfumes/D_S__Durga/steamed-rainbow/reviews/724459");
    assert_eq!(
        (r.brand.as_deref(), r.title.as_deref(), r.date.as_deref()),
        (Some("D.S. & Durga"), Some("Steam, Ozone, Green, and Bright"), Some("5 days ago"))
    );
    assert!(r.text.starts_with("This one is very unique!") && r.text.contains('\n'));
    assert_eq!((r.ratings.scent, r.ratings.durability, r.ratings.sillage, r.awards), (Some(9.0), Some(4.0), Some(5.0), Some(1)));

    let statements = parse::parse_user_statements(&fixture("user_statements.html"), 1).unwrap();
    assert_eq!(statements.items.len(), 1);
    let s = &statements.items[0];
    assert_eq!((s.id, s.perfume_id.as_str(), s.brand.as_deref()), (2797680, "Dior/sauvage-elixir", Some("Dior")));
    assert_eq!(s.url, "https://www.parfumo.com/Perfumes/Dior/sauvage-elixir/statements/2797680");
    assert!(s.text.starts_with("Wore a sample") && s.awards == Some(1));

    assert!(parse::parse_user_reviews(&fixture("auth_user_reviews_empty.html"), 1).unwrap().items.is_empty());
    assert!(parse::parse_user_statements(&fixture("auth_user_statements_empty.html"), 1).unwrap().items.is_empty());
    assert_outdated(parse::parse_user_reviews(&fixture("auth_tracker.html"), 1));
}

#[test]
fn nickname_is_read_from_the_page_header() {
    assert_eq!(parse::parse_own_nick(&fixture("auth_settings_notifications.html")).unwrap(), "testuser");
    assert_outdated(parse::parse_own_nick(&fixture("perfume_sauvage.html")));
}
