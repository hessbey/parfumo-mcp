//! Blogs, Souk and wall (jalon 4): parser tests on HTML recorded anonymously from
//! parfumo.com on 2026-11-12 (`h` tokens replaced by a placeholder), and client tests
//! against a local mock serving the same pages. No network.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    extract::{Query, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use parfumo_client::{BlogFeed, ClientConfig, ClientError, GlobalLimiter, ParfumoClient, SoukKind, parse};

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

const GARBAGE: &str = "<html><head><title>Parfumo</title></head><body><div class=\"main\"><p>redesigned</p></div></body></html>";
const ARTICLE_URL: &str = "https://www.parfumo.com/Users/member309/Blog/Article/zoologist-nightingale-rose-petals-in-a-sunlit-church";

// --- blogs ---

#[test]
fn blog_feed_latest_page() {
    let list = parse::parse_blog_feed(&fixture("blog_feed.html"), 1).unwrap();
    assert_eq!(list.items.len(), 8);
    assert_eq!((list.page, list.total_pages, list.total_items), (1, Some(186), Some(1483)));
    let first = &list.items[0];
    assert_eq!(first.id, "member309/zoologist-nightingale-rose-petals-in-a-sunlit-church");
    assert_eq!(first.url, ARTICLE_URL);
    assert_eq!(first.title, "Zoologist Nightingale. Rose Petals in a Sunlit Church.");
    assert_eq!(first.author, "member309");
    assert_eq!(first.author_url, "https://www.parfumo.com/Users/member309");
    assert_eq!(first.date.as_deref(), Some("1 day ago"));
    assert!(first.text.as_deref().unwrap().starts_with("Oh, what a wonderful brand – Zoologist!"));
    assert_eq!((first.comments, first.awards), (Some(1), Some(3)));
    assert!(first.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/blog_cover/"));
    // A card without cover image and without comment counter.
    let second = &list.items[1];
    assert_eq!(second.id, "member176/honeysuckle-the-white-floral-that-smells-like-banana");
    assert_eq!(second.title, "Honeysuckle: The White Floral That Smells Like Banana?");
    assert_eq!((second.comments, second.awards, second.image_url.as_deref()), (None, Some(1), None));
    assert!(list.items.iter().all(|e| !e.title.is_empty() && e.url.contains("/Blog/Article/")));
}

#[test]
fn blog_feed_top_is_a_single_page() {
    let list = parse::parse_blog_feed(&fixture("blog_top.html"), 1).unwrap();
    assert_eq!(list.items.len(), 8);
    assert_eq!((list.total_pages, list.total_items), (Some(1), Some(8)));
    assert_eq!(list.items[0].id, "member054/update-001-the-new-postbox");
    assert_eq!(list.items[0].awards, Some(7));
}

#[test]
fn user_blog_page_and_empty_blog() {
    let list = parse::parse_user_blog(&fixture("blog_user.html"), 1).unwrap();
    assert_eq!(list.items.len(), 6);
    assert_eq!((list.total_pages, list.total_items), (Some(4), Some(20)));
    assert!(list.items.iter().all(|e| e.author == "member309"));
    assert_eq!(list.items[1].id, "member309/when-you-find-your-rose-and-finally-settle-down");
    assert_eq!(list.items[1].date.as_deref(), Some("4 days ago"));

    // "member240 has not yet published any articles."
    let empty = parse::parse_user_blog(&fixture("blog_user_empty.html"), 1).unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.total_items, Some(0));
}

#[test]
fn blog_article_full_text() {
    let a = parse::parse_blog_article(&fixture("blog_article.html"), ARTICLE_URL).unwrap();
    assert_eq!(a.id, "member309/zoologist-nightingale-rose-petals-in-a-sunlit-church");
    assert_eq!(a.article_id, Some(31005));
    assert_eq!(a.url, ARTICLE_URL);
    assert_eq!(a.title, "Zoologist Nightingale. Rose Petals in a Sunlit Church.");
    assert_eq!(a.author, "member309");
    assert_eq!(a.author_url, "https://www.parfumo.com/Users/member309");
    assert_eq!(a.author_id, Some(1296992));
    assert_eq!(a.blog_title.as_deref(), Some("Embark on a parfume journey"));
    assert_eq!(a.date.as_deref(), Some("1 day ago - 10/11/2026"));
    assert_eq!((a.comments, a.awards), (Some(1), Some(3)));
    // Paragraphs survive as blank-line separated blocks; figures leave no text.
    let paragraphs: Vec<&str> = a.text.split("\n\n").collect();
    assert_eq!(paragraphs.len(), 6);
    assert!(paragraphs[0].starts_with("Oh, what a wonderful brand – Zoologist!"));
    assert_eq!(paragraphs[1], "Now, Nightingale.");
    assert_eq!(paragraphs[5], "Absolutely beautiful. Pure, tender, bright. Feminine.");
    assert_eq!(a.images.len(), 2);
    assert!(a.images[0].starts_with("https://www.parfumo.com/blog_images/images/"));
    // The "More articles" cards below the article are not part of it.
    assert!(!a.text.contains("La Fille de Berlin"));
    assert_eq!(a.comment_list.len(), 1);
    assert_eq!(a.comment_list[0].id, 22329337);
    assert_eq!(a.comment_list[0].author, "member228");
    assert!(a.comment_list[0].text.starts_with("Great post."));
}

// --- Souk ---

#[test]
fn souk_latest_offers_requests_and_sharings() {
    let offers = parse::parse_souk_listing(&fixture("souk_offers.html"), 1).unwrap();
    assert_eq!(offers.items.len(), 16);
    assert_eq!((offers.total_pages, offers.total_items), (Some(1845), Some(29507)));
    let first = &offers.items[0];
    assert_eq!((first.id, first.kind), (6782002, SoukKind::Offers));
    assert_eq!(first.url, "https://www.parfumo.com/Users/member240/Souk/Item/6782002");
    assert_eq!(first.perfume_name, "Spicy Road");
    assert_eq!(first.brand.as_deref(), Some("Une Nuit Nomade"));
    assert_eq!(first.author, "member240");
    assert_eq!(first.author_url, "https://www.parfumo.com/Users/member240");
    assert_eq!(first.country_code.as_deref(), Some("de"));
    assert_eq!(first.date.as_deref(), Some("Just now"));
    assert!(first.has_photos);
    assert_eq!((first.perfume_id.as_deref(), first.condition.as_deref()), (None, None), "lists carry no details");
    assert_eq!(offers.items[3].perfume_name, "Must de Cartier");
    assert_eq!(offers.items[3].concentration.as_deref(), Some("Parfum"));

    let requests = parse::parse_souk_listing(&fixture("souk_requests.html"), 1).unwrap();
    assert_eq!(requests.items.len(), 16);
    assert_eq!((requests.total_pages, requests.total_items), (Some(938), Some(15003)));
    assert!(requests.items.iter().all(|i| i.kind == SoukKind::Requests && !i.has_photos));
    assert_eq!(requests.items[1].author, "member293");
    assert_eq!(requests.items[1].country_code.as_deref(), Some("at"));

    let sharings = parse::parse_souk_listing(&fixture("souk_sharings.html"), 1).unwrap();
    assert_eq!(sharings.items.len(), 16);
    assert_eq!((sharings.total_pages, sharings.total_items), (Some(81), Some(1281)));
    let split = &sharings.items[1];
    assert_eq!((split.id, split.kind), (325192, SoukKind::Sharings));
    assert_eq!(split.url, "https://www.parfumo.com/Users/member253/Souk/Sharing/325192");
    assert_eq!(split.perfume_name, "Aloha");
    assert_eq!(split.size.as_deref(), Some("16 ml left"));
    assert_eq!(split.participants, Some(18));
    assert_eq!(sharings.items[0].participants, None);
}

#[test]
fn souk_search_results_and_empty_search() {
    let offers = parse::parse_souk_listing(&fixture("souk_search.html"), 1).unwrap();
    assert_eq!(offers.items.len(), 20);
    assert_eq!((offers.total_pages, offers.total_items), (Some(5), Some(94)));
    let first = &offers.items[0];
    assert_eq!((first.id, first.kind), (6780925, SoukKind::Offers));
    assert_eq!(first.perfume_name, "Eau Sauvage Parfum (2017)");
    assert_eq!(first.brand.as_deref(), Some("Dior"));
    assert_eq!(first.author, "member223");
    assert_eq!(first.item_type.as_deref(), Some("Bottle"));
    assert_eq!(first.date.as_deref(), Some("19 hours ago"));
    assert_eq!(offers.items[1].concentration.as_deref(), Some("Eau de Parfum"));

    let requests = parse::parse_souk_listing(&fixture("souk_search_requests.html"), 1).unwrap();
    assert_eq!(requests.items.len(), 20);
    assert_eq!((requests.total_pages, requests.total_items), (Some(2), Some(30)));
    assert!(requests.items.iter().all(|i| i.kind == SoukKind::Requests));
    assert_eq!(requests.items[1].item_type.as_deref(), Some("Sample"));
    assert_eq!(requests.items[1].author, "member255");

    let sharings = parse::parse_souk_listing(&fixture("souk_search_sharings.html"), 1).unwrap();
    assert_eq!(sharings.items.len(), 20);
    assert_eq!((sharings.total_pages, sharings.total_items), (Some(12), Some(240)));
    assert!(sharings.items.iter().all(|i| i.kind == SoukKind::Sharings && i.item_type.is_none()));
    assert_eq!(sharings.items[1].status, ["Complete"]);
    assert_eq!((sharings.items[1].size.as_deref(), sharings.items[1].participants), (None, Some(3)));

    // "Sorry, we couldn't find any Offers or Requests for your search term."
    let empty = parse::parse_souk_listing(&fixture("souk_search_empty.html"), 1).unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.total_items, Some(0));
}

#[test]
fn souk_member_tabs() {
    let (items, tabs, pages) = parse::parse_user_souk(&fixture("souk_user.html")).unwrap();
    assert_eq!(items.len(), 10);
    assert!(items.iter().all(|i| i.kind == SoukKind::Offers && i.author == "member240"));
    assert_eq!(items[0].id, 6782002);
    assert_eq!(items[6].perfume_name, "Georges et Moi");
    assert_eq!(items[6].concentration.as_deref(), Some("Solid Perfume"));
    // Tabs with a counter: offers (the page itself) and requests; samples and splits are empty.
    assert_eq!(tabs, ["/Users/member240/Souk", "/Users/member240/Souk/Requests"]);
    assert_eq!(pages, 1);

    let (items, _, _) = parse::parse_user_souk(&fixture("souk_user_requests.html")).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].id, items[0].kind), (6740948, SoukKind::Requests));
    assert_eq!(items[0].perfume_name, "Orange Flowers");

    // "member240 is not currently hosting any sharings."
    let (items, tabs, _) = parse::parse_user_souk(&fixture("souk_user_empty.html")).unwrap();
    assert!(items.is_empty());
    assert_eq!(tabs.len(), 2);
}

#[test]
fn souk_entry_pages() {
    let url = "https://www.parfumo.com/Users/member223/Souk/Item/6780925";
    let offer = parse::parse_souk_item(&fixture("souk_item_offer.html"), url).unwrap();
    assert_eq!((offer.id, offer.kind), (6780925, SoukKind::Offers));
    assert_eq!(offer.url, url);
    assert_eq!(offer.perfume_name, "Eau Sauvage Parfum (2017)");
    assert_eq!(offer.brand.as_deref(), Some("Dior"));
    assert_eq!(offer.perfume_id.as_deref(), Some("Dior/Eau_Sauvage_Parfum_2017"));
    assert_eq!(offer.perfume_url.as_deref(), Some("https://www.parfumo.com/Perfumes/Dior/Eau_Sauvage_Parfum_2017"));
    assert_eq!(offer.author, "member223");
    assert_eq!(offer.author_url, "https://www.parfumo.com/Users/member223");
    assert_eq!(offer.author_id, Some(776056));
    assert_eq!((offer.country.as_deref(), offer.country_code.as_deref()), (Some("Germany"), Some("de")));
    assert_eq!(offer.item_type.as_deref(), Some("Bottle"));
    assert_eq!(offer.trade.as_deref(), Some("Purchase"));
    assert_eq!(offer.preference.as_deref(), Some("Purchase preferred"));
    assert_eq!(offer.condition.as_deref(), Some("Second Hand"));
    assert_eq!(offer.size.as_deref(), Some("30 ml of 100 ml"));
    assert_eq!(offer.size_label.as_deref(), Some("Current content"));
    assert_eq!(offer.note.as_deref(), Some("Aus dem Release von 2017\nBezahlung p/p freunde und Familiy"));
    assert_eq!(offer.date.as_deref(), Some("19 hours ago"));
    assert_eq!(offer.photos.len(), 2);
    assert!(offer.has_photos && offer.photos[0].starts_with("https://media.parfumo.com/souk/"));
    assert_eq!(offer.details.len(), 1);
    assert_eq!((offer.details[0].label.as_str(), offer.details[0].value.as_str()), ("Purchase Date", "2017"));
    assert_eq!((offer.price.as_deref(), offer.shipping.as_deref(), offer.swap_for.as_deref()), (None, None, None));

    let url = "https://www.parfumo.com/Users/member054/Souk/Item/6782001";
    let request = parse::parse_souk_item(&fixture("souk_item_request.html"), url).unwrap();
    assert_eq!((request.id, request.kind), (6782001, SoukKind::Requests));
    assert_eq!(request.perfume_id.as_deref(), Some("Serge_Lutens/Un_Bois_Vanille"));
    assert_eq!(request.author_id, Some(1));
    assert_eq!(request.item_type.as_deref(), Some("Bottle"));
    assert_eq!(request.condition.as_deref(), Some("Any"));
    assert_eq!(request.price.as_deref(), Some("Bietet mir etwas an :-)"));
    assert_eq!((request.size.as_deref(), request.size_label.as_deref()), (Some("50 ml"), Some("Minimum Content")));
    assert!(!request.has_photos && request.note.is_none());

    let url = "https://www.parfumo.com/Users/member253/Souk/Sharing/325192";
    let split = parse::parse_souk_item(&fixture("souk_item_sharing.html"), url).unwrap();
    assert_eq!((split.id, split.kind), (325192, SoukKind::Sharings));
    assert_eq!(split.perfume_name, "Aloha");
    assert_eq!(split.perfume_id.as_deref(), Some("marc-gebauer/aloha"));
    assert_eq!(split.author_id, Some(1229075));
    assert_eq!(split.participants, Some(18));
    assert_eq!(split.price.as_deref(), Some("€2,30 / ml"));
    assert!(split.shipping.as_deref().unwrap().starts_with("3€"));
    assert_eq!((split.size.as_deref(), split.size_label.as_deref()), (Some("16 ml left"), Some("of 82 ml")));
    assert_eq!(split.status, ["Closing in 1 month", "Updated 4 minutes ago"]);
    assert_eq!(split.date.as_deref(), Some("16 hours ago"));
    let note = split.note.as_deref().unwrap();
    assert!(note.starts_with("Kombination Sharing's möglich!\n- Nishane"));
    assert!(note.contains("\n\n*ausgenommen"));
    let labels: Vec<&str> = split.details.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels, ["Amount to Split", "Decant", "Purchase details"]);
    assert_eq!(split.details[0].value, "82 ml of 100 ml bottle");
    assert_eq!(split.photos.len(), 1);
}

// --- wall ---

#[test]
fn wall_page_with_reply_and_owner_id() {
    let wall = parse::parse_wall(&fixture("wall_page.html"), 1).unwrap();
    assert_eq!(wall.owner, "member054");
    assert_eq!(wall.owner_url, "https://www.parfumo.com/Users/member054");
    assert_eq!(wall.owner_id, Some(1));
    assert_eq!(wall.entries.items.len(), 20);
    assert_eq!((wall.entries.page, wall.entries.total_pages, wall.entries.total_items), (1, Some(23), Some(451)));
    let first = &wall.entries.items[0];
    assert_eq!(first.id, 22276232);
    assert_eq!(first.author, "member045");
    assert_eq!(first.author_url, "https://www.parfumo.com/Users/member045");
    assert_eq!(first.date.as_deref(), Some("18 days ago"));
    assert_eq!(first.text, "Notiere: Herzlichen Dank!!");
    assert!(first.replies.is_empty());
    assert_eq!(wall.entries.items[1].text.lines().count(), 4, "line breaks are kept");
    // An entry answered by the owner: the answer is nested, not a twenty-first entry.
    let answered = &wall.entries.items[13];
    assert_eq!((answered.id, answered.author.as_str()), (12940562, "member058"));
    assert!(answered.text.starts_with("Dieser ResearchBot ist echt fleißig!"));
    assert_eq!(answered.replies.len(), 1);
    assert_eq!(answered.replies[0].id, 12940607);
    assert_eq!(answered.replies[0].author, "member054");
    assert_eq!(answered.replies[0].date.as_deref(), Some("3 years ago"));
    assert_eq!(answered.replies[0].text, "Ich hoffe, ich kann ihm noch mehr beibringen!");
    assert_eq!(wall.entries.items.iter().map(|e| e.replies.len()).sum::<usize>(), 1);

    let wall = parse::parse_wall(&fixture("wall_member.html"), 1).unwrap();
    assert_eq!((wall.owner.as_str(), wall.owner_id), ("member240", Some(71772)));
    assert_eq!(wall.entries.items.len(), 20);
    assert_eq!((wall.entries.total_pages, wall.entries.total_items), (Some(15), Some(292)));
    assert_eq!(wall.entries.items[0].author, "member078");

    // A counter starting at 1 on another page than the first means the site fell back to
    // page 1 (its behaviour past the end of other listings): no entry is reported.
    let past = parse::parse_wall(&fixture("wall_member.html"), 40).unwrap();
    assert!(past.entries.items.is_empty());
    assert_eq!(past.entries.total_items, Some(292));
}

// --- structure changes ---

#[test]
fn garbage_pages_are_reported_as_outdated() {
    assert_outdated(parse::parse_blog_feed(GARBAGE, 1));
    assert_outdated(parse::parse_user_blog(GARBAGE, 1));
    assert_outdated(parse::parse_blog_article(GARBAGE, ARTICLE_URL));
    assert_outdated(parse::parse_souk_listing(GARBAGE, 1));
    assert_outdated(parse::parse_user_souk(GARBAGE));
    assert_outdated(parse::parse_souk_item(GARBAGE, "https://www.parfumo.com/Users/member253/Souk/Sharing/325192"));
    assert_outdated(parse::parse_wall(GARBAGE, 1));
    // A page of another kind is not mistaken for an empty list.
    assert_outdated(parse::parse_blog_feed(&fixture("souk_offers.html"), 1));
    assert_outdated(parse::parse_souk_listing(&fixture("blog_feed.html"), 1));
    assert_outdated(parse::parse_user_souk(&fixture("blog_user.html")));
    assert_outdated(parse::parse_souk_item(&fixture("souk_user.html"), "https://www.parfumo.com/Users/member240/Souk/Item/6782002"));
    assert_outdated(parse::parse_blog_article(&fixture("blog_user.html"), ARTICLE_URL));
    // Cards whose markup changed are an error, not a shorter list.
    let broken = fixture("souk_offers.html").replace("class=\"souk-card\"", "class=\"souk-tile\"");
    assert_outdated(parse::parse_souk_listing(&broken, 1));
    let broken = fixture("wall_page.html").replace("comment-text-", "comment-body-");
    assert_outdated(parse::parse_wall(&broken, 1));
}

// --- client against a local mock ---

#[derive(Default)]
struct Mock {
    hits: Mutex<Vec<String>>,
}

async fn page(State(m): State<Arc<Mock>>, uri: Uri, Query(q): Query<HashMap<String, String>>) -> Response {
    m.hits.lock().unwrap().push(uri.to_string());
    let name = match uri.path() {
        "/blogs" => "blog_feed.html",
        "/blogs/top" => "blog_top.html",
        "/Users/member309/Blog" => "blog_user.html",
        "/Users/member309/Blog/Article/zoologist-nightingale-rose-petals-in-a-sunlit-church" => "blog_article.html",
        "/Souks/Requests" => "souk_requests.html",
        "/s_souk.php" => match (q.get("s_type").map(String::as_str), q.get("filter").map(String::as_str)) {
            (Some("offer"), Some("Sauvage")) => "souk_search.html",
            (Some("need"), Some("Sauvage")) => "souk_search_requests.html",
            _ => "souk_search_empty.html",
        },
        "/Users/member240/Souk" => "souk_user.html",
        "/Users/member240/Souk/Requests" => "souk_user_requests.html",
        "/Users/member223/Souk/Item/6780925" => "souk_item_offer.html",
        "/Users/member240/Wall" => "wall_member.html",
        _ => return (StatusCode::NOT_FOUND, fixture("notfound.html")).into_response(),
    };
    fixture(name).into_response()
}

async fn start() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let app = Router::new().fallback(get(page)).with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn client(base: &str) -> ParfumoClient {
    let cfg = ClientConfig {
        base_url: base.to_string(),
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        max_retries: 0,
        backoff_base: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    };
    ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap()
}

fn assert_invalid<T: std::fmt::Debug>(result: Result<T, ClientError>) {
    assert!(matches!(result, Err(ClientError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
}

#[tokio::test]
async fn client_reads_blogs() {
    let (base, mock) = start().await;
    let c = client(&base);

    assert_eq!(c.blog_feed(BlogFeed::Latest, 1).await.unwrap().items.len(), 8);
    assert_eq!(c.blog_feed(BlogFeed::default(), 3).await.unwrap().page, 3);
    assert_eq!(mock.hits.lock().unwrap().last().unwrap(), "/blogs?current_page=3");
    // The weekly lists have one page: page 2 is answered without a request.
    let hits = mock.hits.lock().unwrap().len();
    let top = c.blog_feed(BlogFeed::Top, 2).await.unwrap();
    assert!(top.items.is_empty());
    assert_eq!(top.total_pages, Some(1));
    assert_eq!(mock.hits.lock().unwrap().len(), hits);

    // A member is named by nickname, path or URL on the Parfumo host.
    for nick in ["member309", "/Users/member309", "Users/member309/Blog", &format!("{base}/Users/member309/Wall")] {
        let blog = c.user_blog(nick, 1).await.unwrap();
        assert_eq!(blog.total_items, Some(20), "{nick}");
    }
    assert_invalid(c.user_blog("https://evil.example/Users/member309", 1).await);
    assert_invalid(c.user_blog("/Perfumes/Dior", 1).await);
    assert_invalid(c.user_blog("  ", 1).await);
    assert_invalid(c.user_blog("../admin", 1).await);
    assert_eq!(c.user_blog("NoSuchMember", 1).await, Err(ClientError::NotFound("/Users/NoSuchMember/Blog".into())));

    let path = "/Users/member309/Blog/Article/zoologist-nightingale-rose-petals-in-a-sunlit-church";
    for article in [path.to_string(), format!("{base}{path}?utm=1"), path.trim_start_matches("/Users/").to_string()] {
        let a = c.blog_article(&article).await.unwrap();
        assert_eq!(a.article_id, Some(31005), "{article}");
        assert_eq!(a.url, format!("{base}{path}"));
    }
    assert_invalid(c.blog_article("/Users/member309/Blog").await);
    assert_invalid(c.blog_article(&format!("https://www.parfumo.de{path}")).await);
}

#[tokio::test]
async fn client_reads_the_souk() {
    let (base, mock) = start().await;
    let c = client(&base);

    let found = c.souk_search(SoukKind::Offers, Some(" Sauvage "), 1).await.unwrap();
    assert_eq!(found.total_items, Some(94));
    assert_eq!(mock.hits.lock().unwrap().last().unwrap(), "/s_souk.php?s_type=offer&filter=Sauvage&as=1&cs=1&img=1");
    let found = c.souk_search(SoukKind::Requests, Some("Sauvage"), 2).await.unwrap();
    assert!(found.items.is_empty(), "the mock answers page 1 again, as the site does past the end");
    assert_eq!(mock.hits.lock().unwrap().last().unwrap(), "/s_souk.php?s_type=need&filter=Sauvage&as=1&cs=1&img=1&current_page=2");
    let none = c.souk_search(SoukKind::Sharings, Some("zzqqxx"), 1).await.unwrap();
    assert_eq!((none.items.len(), none.total_items), (0, Some(0)));
    assert!(mock.hits.lock().unwrap().last().unwrap().starts_with("/s_souk.php?s_type=sharing&"));
    // Without a query the latest entries of the section are listed.
    for query in [None, Some("  ")] {
        let latest = c.souk_search(SoukKind::Requests, query, 1).await.unwrap();
        assert_eq!(latest.items.len(), 16);
        assert_eq!(mock.hits.lock().unwrap().last().unwrap(), "/Souks/Requests");
    }
    assert_invalid(c.souk_search(SoukKind::Offers, Some(&"x".repeat(161)), 1).await);

    // A member's Souk: the offers page, then the only other tab with a counter.
    let before = mock.hits.lock().unwrap().len();
    let items = c.user_souk("MEMBER253").await;
    assert_eq!(items, Err(ClientError::NotFound("/Users/MEMBER253/Souk".into())), "the mock is case sensitive");
    let items = c.user_souk(&format!("{base}/Users/member240/Souk")).await.unwrap();
    assert_eq!(items.len(), 11);
    assert_eq!(items.iter().filter(|i| i.kind == SoukKind::Requests).count(), 1);
    assert_eq!(items[10].id, 6740948);
    let hits = mock.hits.lock().unwrap()[before + 1..].to_vec();
    assert_eq!(hits, ["/Users/member240/Souk", "/Users/member240/Souk/Requests"]);

    let entry = c.souk_item("member223/Souk/Item/6780925").await.unwrap();
    assert_eq!(entry.condition.as_deref(), Some("Second Hand"));
    assert_eq!(entry.url, format!("{base}/Users/member223/Souk/Item/6780925"));
    assert_invalid(c.souk_item("/Users/member223/Souk").await);
    assert_invalid(c.souk_item("https://example.org/Users/member223/Souk/Item/6780925").await);
    assert!(matches!(c.souk_item("/Users/member223/Souk/Item/1").await, Err(ClientError::NotFound(_))));
}

#[tokio::test]
async fn client_reads_a_wall() {
    let (base, mock) = start().await;
    let c = client(&base);
    let wall = c.user_wall("member240", 1).await.unwrap();
    assert_eq!((wall.owner_id, wall.entries.items.len()), (Some(71772), 20));
    let wall = c.user_wall(&format!("{base}/Users/member240"), 2).await.unwrap();
    assert_eq!(mock.hits.lock().unwrap().last().unwrap(), "/Users/member240/Wall?current_page=2");
    assert_eq!(wall.entries.page, 2);
    assert_invalid(c.user_wall("https://www.parfumo.de/Users/member240", 1).await);
    assert!(matches!(c.user_wall("NoSuchMember", 1).await, Err(ClientError::NotFound(_))));
}
