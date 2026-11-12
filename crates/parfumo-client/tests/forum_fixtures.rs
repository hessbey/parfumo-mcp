//! Forum and communities (jalon 4): parser tests on HTML recorded anonymously from
//! parfumo.com on 2026-11-12, and client routing against a local mock. No network.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Router, extract::State, http::Uri, response::IntoResponse};
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, parse};

const SITE: &str = "https://www.parfumo.com";
const TOPIC: &str = "your-favourite-discontinued-perfumes";
const POST_SLUG: &str = "hii-i-was-so-stoked-to-see-that-this-community-existed-because-i-am";

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

fn assert_invalid<T: std::fmt::Debug>(result: Result<T, ClientError>) {
    assert!(matches!(result, Err(ClientError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
}

/// The first `quarters` quarters of a fixture, as a body cut short would be.
fn truncated(name: &str, quarters: usize) -> String {
    let html = fixture(name);
    let mut cut = html.len() * quarters / 4;
    while !html.is_char_boundary(cut) {
        cut -= 1;
    }
    html[..cut].to_string()
}

// ---------------------------------------------------------------------------
// Forum
// ---------------------------------------------------------------------------

#[test]
fn forum_home_lists_categories_by_section() {
    let cats = parse::parse_forum_home(&fixture("forum_home.html")).unwrap();
    assert_eq!(cats.len(), 8);
    let slugs: Vec<&str> = cats.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(
        slugs,
        [
            "perfumes-brands",
            "perfumes-by-note-accord-genre",
            "perfume-discussions",
            "perfume-recommendations",
            "parfumo-community",
            "off-topic",
            "parfumo-research",
            "swap-giveaway-free-samples",
        ]
    );
    let first = &cats[0];
    assert_eq!(first.forum_id, Some(7));
    assert_eq!(first.name, "Perfumes & Brands");
    assert_eq!(first.description.as_deref(), Some("Let's talk perfume."));
    assert_eq!(first.section.as_deref(), Some("The World of Fragrances"));
    assert_eq!(first.topics, Some(857));
    assert_eq!(first.topics_today, Some(2));
    assert_eq!(first.url, format!("{SITE}/forums/perfumes-brands"));
    let ids: Vec<Option<u64>> = cats.iter().map(|c| c.forum_id).collect();
    assert_eq!(ids, [Some(7), Some(19), Some(18), Some(12), Some(8), Some(5), Some(14), Some(4)]);
    assert_eq!(cats[1].topics_today, None);
    assert_eq!(cats[4].section.as_deref(), Some("The Community"));
    assert_eq!(cats[7].section.as_deref(), Some("Parfumo Souk"));
    assert_eq!(cats[7].topics, Some(23));
}

#[test]
fn forum_category_lists_topics() {
    let page = parse::parse_forum_category(&fixture("forum_category.html"), 1).unwrap();
    let cat = &page.category;
    assert_eq!(cat.slug, "perfumes-brands");
    assert_eq!(cat.forum_id, Some(7));
    assert_eq!(cat.name, "Perfumes & Brands");
    assert_eq!(cat.description.as_deref(), Some("Let's talk perfume."));
    assert_eq!(cat.topics, Some(857));
    assert_eq!(cat.url, format!("{SITE}/forums/perfumes-brands"));

    let topics = &page.topics;
    assert_eq!(topics.items.len(), 50);
    assert_eq!((topics.page, topics.total_pages, topics.total_items), (1, Some(18), Some(857)));
    assert_eq!(topics.items.iter().filter(|t| t.pinned).count(), 1);

    let sticky = &topics.items[0];
    assert_eq!(sticky.slug, "note-for-founders-of-new-perfume-brands");
    assert_eq!(sticky.title, "Note for founders of new perfume brands");
    assert!(sticky.pinned && sticky.locked);
    assert_eq!(sticky.replies, Some(0));
    assert_eq!(sticky.pages, None);

    let t = &topics.items[1];
    assert_eq!(t.slug, "scent-of-the-day");
    assert_eq!(t.title, "Scent of the Day");
    assert_eq!(t.url, format!("{SITE}/forums/topic/scent-of-the-day"));
    assert_eq!(t.author.as_deref(), Some("member159"));
    assert_eq!(t.author_url.as_deref(), Some("https://www.parfumo.com/Users/member159"));
    assert_eq!(t.replies, Some(16488));
    assert_eq!(t.pages, Some(660));
    assert_eq!(t.last_post_id, Some(238772));
    assert_eq!(t.last_post_url.as_deref(), Some("https://www.parfumo.com/forums/post/238772#238772"));
    assert_eq!(t.last_post_author.as_deref(), Some("member203"));
    assert_eq!(t.last_post_date.as_deref(), Some("8 hours ago"));
    assert!(!t.pinned && !t.locked);
    assert!(topics.items.iter().any(|t| t.slug == TOPIC && t.replies == Some(61) && t.pages == Some(3)));
}

#[test]
fn forum_topic_first_page() {
    let thread = parse::parse_forum_thread(&fixture("forum_topic.html"), 1).unwrap();
    let topic = &thread.topic;
    assert_eq!(topic.slug, TOPIC);
    assert_eq!(topic.topic_id, 29437);
    assert_eq!(topic.forum_id, Some(7));
    assert_eq!(topic.title, "Your Favourite Discontinued Perfumes ?");
    assert_eq!(topic.url, format!("{SITE}/forums/topic/{TOPIC}"));
    assert_eq!(topic.category_slug.as_deref(), Some("perfumes-brands"));
    assert_eq!(topic.category_name.as_deref(), Some("Perfumes & Brands"));
    assert_eq!(topic.category_url.as_deref(), Some("https://www.parfumo.com/forums/perfumes-brands"));

    let posts = &thread.posts;
    assert_eq!(posts.items.len(), 25);
    assert_eq!((posts.page, posts.total_pages, posts.total_items), (1, Some(3), Some(62)));

    let opening = &posts.items[0];
    assert_eq!(opening.id, 189544);
    assert_eq!(opening.url, format!("{SITE}/forums/post/189544#189544"));
    assert_eq!(opening.author, "ExUser");
    assert_eq!(opening.author_url.as_deref(), Some("https://www.parfumo.com/Users/ExUser"));
    assert_eq!(opening.date.as_deref(), Some("1 year ago"));
    assert_eq!(opening.date_iso.as_deref(), Some("2025-06-25T20:31:15+02:00"));
    assert_eq!(opening.title.as_deref(), Some("Your Favourite Discontinued Perfumes ?"));
    assert_eq!(opening.thanks, Some(1));
    // Paragraphs survive as line breaks; perfume links keep their label.
    assert!(opening.text.starts_with("Here is my list -\n\nVendetta Eau de Toilette / Vendetta Parfum (This is also"));
    assert!(opening.text.contains("\n\nRose Cardin\n\nMy two all time favourite perfumes"));

    let reply = &posts.items[1];
    assert_eq!((reply.id, reply.author.as_str(), reply.title.as_deref(), reply.thanks), (189574, "member208", None, Some(1)));
    assert_eq!(
        reply.text,
        "L'Eau de Monteil\n\nEscada Margaretha Ley Parfum\n\nDewberry Perfume Oil\n\nSummer by Kenzo\n\nWoman III (1986) Eau de Toilette"
    );

    // A quoted post stays inline: nickname of the quoted member, then the quote, then the answer.
    let quoting = &posts.items[2];
    assert_eq!(quoting.id, 189586);
    assert_eq!(quoting.thanks, Some(2));
    assert!(quoting.text.starts_with("member208\n\nL'Eau de Monteil\n\n"));
    assert!(quoting.text.contains("\n\n@member208\nGood to see Escada (1990) Eau de Parfum there"));
    assert_eq!(posts.items[24].id, 192577);
}

#[test]
fn forum_topic_second_page_reports_the_page_shown() {
    // Parsed as if page 1 had been asked (what a `/forums/post/{id}` read does):
    // the page number comes from the page itself.
    let thread = parse::parse_forum_thread(&fixture("forum_topic_page2.html"), 1).unwrap();
    assert_eq!(thread.topic.slug, TOPIC);
    assert_eq!(thread.topic.topic_id, 29437);
    assert_eq!(thread.topic.title, "Your Favourite Discontinued Perfumes ?");
    assert_eq!(thread.topic.url, format!("{SITE}/forums/topic/{TOPIC}"));
    let posts = &thread.posts;
    assert_eq!(posts.items.len(), 25);
    assert_eq!((posts.page, posts.total_pages, posts.total_items), (2, Some(3), Some(62)));
    assert_eq!(posts.items[0].id, 192580);
    assert_eq!(posts.items[0].author, "member102");
    assert_eq!(posts.items[0].title, None);
    assert_eq!(posts.items[0].thanks, Some(0));
    assert_eq!(posts.items[1].author, "member256");
    assert!(posts.items[1].text.starts_with("Omnia - I think I would have used it for life"));
    assert!(posts.items.iter().any(|p| p.id == 213001));
    assert_eq!(posts.items[24].id, 225865);
}

#[test]
fn forum_search_lists_matching_posts() {
    let hits = parse::parse_forum_search(&fixture("forum_search.html"), 1).unwrap();
    assert_eq!(hits.items.len(), 20);
    assert_eq!((hits.page, hits.total_pages, hits.total_items), (1, Some(50), Some(993)));
    let hit = &hits.items[0];
    assert_eq!(hit.post_id, 238548);
    assert_eq!(hit.url, format!("{SITE}/forums/post/238548#238548"));
    assert_eq!(hit.title, "Your most satisfying fragrance purchase");
    assert_eq!(hit.author.as_deref(), Some("member179"));
    assert_eq!(hit.author_url.as_deref(), Some("https://www.parfumo.com/Users/member179"));
    assert_eq!(hit.category_slug.as_deref(), Some("parfumo-community"));
    assert_eq!(hit.category_name.as_deref(), Some("Parfumo Community"));
    assert_eq!(hit.date.as_deref(), Some("5 days ago"));
    assert!(hit.text.as_deref().unwrap().starts_with("…Original Vetiver | Creed\" I wore the sample"));
    // Highlighted words lose their markup, in titles too.
    assert_eq!(hits.items[3].title, "What is your favorite vetiver fragrance?");
    assert!(hits.items.iter().all(|h| h.post_id > 0 && h.author.is_some() && h.category_slug.is_some()));
}

#[test]
fn forum_search_without_result_is_empty_not_an_error() {
    let hits = parse::parse_forum_search(&fixture("forum_search_empty.html"), 1).unwrap();
    assert!(hits.items.is_empty());
    assert_eq!((hits.page, hits.total_pages, hits.total_items), (1, Some(0), Some(0)));
}

// ---------------------------------------------------------------------------
// Communities
// ---------------------------------------------------------------------------

#[test]
fn communities_directory_page() {
    let page = parse::parse_communities(&fixture("communities.html"), 1).unwrap();
    assert_eq!(page.items.len(), 12);
    assert_eq!((page.page, page.total_pages, page.total_items), (1, Some(9), Some(105)));
    let c = &page.items[0];
    assert_eq!(c.slug, "scent-of-the-day");
    assert_eq!(c.name, "Scent of the Day");
    assert_eq!(c.tagline.as_deref(), Some("What are you wearing today?"));
    assert_eq!(c.description.as_deref(), Some("Share with the community which fragrance you chose today!"));
    assert_eq!((c.members, c.posts), (Some(198), Some(243)));
    assert_eq!(c.access, None);
    assert_eq!(c.group_id, None);
    assert_eq!(c.url, format!("{SITE}/communities/scent-of-the-day"));
    let invite = &page.items[1];
    assert_eq!(invite.slug, "perfume-trading-usa");
    assert_eq!(invite.access.as_deref(), Some("Invite-only"));
    assert_eq!((invite.members, invite.posts), (Some(17), Some(9)));
    assert!(invite.description.as_deref().unwrap().contains("Here's a group"));
    assert_eq!(page.items[2].name, "Testing 1, 2, 3...");
}

#[test]
fn community_page_with_posts() {
    let page = parse::parse_community(&fixture("community.html"), 1).unwrap();
    let c = &page.community;
    assert_eq!(c.slug, "scent-of-the-day");
    assert_eq!(c.group_id, Some(74));
    assert_eq!(c.name, "Scent of the Day");
    assert_eq!(c.tagline.as_deref(), Some("What are you wearing today?"));
    assert_eq!(c.description.as_deref(), Some("Share with the community which fragrance you chose today!"));
    assert_eq!((c.members, c.posts), (Some(198), Some(243)));
    assert_eq!(c.access.as_deref(), Some("Public"));
    assert_eq!(c.tags, ["sotd"]);
    assert_eq!(c.url, format!("{SITE}/communities/scent-of-the-day"));

    let posts = &page.posts;
    assert_eq!(posts.items.len(), 20);
    assert_eq!((posts.page, posts.total_pages, posts.total_items), (1, Some(13), Some(243)));
    let p = &posts.items[0];
    assert_eq!(p.slug, "amazing-option-year-round-that-works-really-well-for-office");
    assert_eq!(p.id, Some(3601));
    assert_eq!(p.community_slug, "scent-of-the-day");
    assert_eq!(p.url, format!("{SITE}/communities/scent-of-the-day/post/{}", p.slug));
    assert_eq!(p.author.as_deref(), Some("member263"));
    assert_eq!(p.author_url.as_deref(), Some("https://www.parfumo.com/Users/member263"));
    assert_eq!(p.date.as_deref(), Some("2 hours ago"));
    assert_eq!(p.title, None);
    assert_eq!(p.text, "Amazing option year-round that works really well for office");
    assert_eq!(p.perfumes, ["Serious | Al Majed Oud"]);
    assert_eq!(p.votes, Some(0));
    assert_eq!(p.comments, None);
    assert_eq!(posts.items[1].perfumes, ["Danger | Roja Parfums", "1969 | Histoires de Parfums"]);
    assert_eq!(posts.items[1].votes, Some(1));
    // Titles are optional in a community feed.
    assert_eq!(posts.items.iter().filter(|p| p.title.is_some()).count(), 3);
    let titled = posts.items.iter().find(|p| p.slug == "nom-nom").unwrap();
    assert_eq!(titled.title.as_deref(), Some("Nom Nom"));
    assert_eq!(titled.text, "Wanted to smell like berries and cream today <3");
}

#[test]
fn community_post_page_has_full_text_but_no_comments() {
    let page = parse::parse_community_post(&fixture("community_post.html")).unwrap();
    assert_eq!(page.community.slug, "the-slightly-haunted-dollhouse");
    assert_eq!(page.community.name, "The Slightly Haunted Dollhouse");
    assert_eq!(page.community.url, format!("{SITE}/communities/the-slightly-haunted-dollhouse"));
    let p = &page.post;
    assert_eq!(p.slug, POST_SLUG);
    assert_eq!(p.id, Some(3592));
    assert_eq!(p.community_slug, "the-slightly-haunted-dollhouse");
    assert_eq!(p.url, format!("{SITE}/communities/the-slightly-haunted-dollhouse/post/{POST_SLUG}"));
    assert_eq!(p.title.as_deref(), Some("Hii! I was so stoked to see that this community existed because I am…"));
    assert_eq!(p.author.as_deref(), Some("member318"));
    assert_eq!(p.author_url.as_deref(), Some("https://www.parfumo.com/Users/member318"));
    assert_eq!(p.date.as_deref(), Some("5 hours ago"));
    assert_eq!(p.date_iso.as_deref(), Some("2026-11-12T06:04:46+01:00"));
    assert!(p.text.starts_with("Hii! I was so stoked to see that this community existed because I am OBSESSED"));
    assert!(p.text.contains("sampling journey ^_^\n\nI first have to shout out Kewpie Doll | Pearfat Parfum"));
    assert!(p.text.ends_with("rubber/plastic/vinyl notes ^_^"));
    assert_eq!(p.perfumes.len(), 7);
    assert_eq!(p.perfumes[0], "Kewpie Doll | Pearfat Parfum");
    assert_eq!(p.perfumes[5], "My Doll's Makeup | Ataraxia");
    assert_eq!(p.votes, Some(2));
    assert_eq!(p.comments, Some(0));
    assert_eq!(page.warnings.len(), 1);
    assert!(page.warnings[0].contains("comments"));
}

// ---------------------------------------------------------------------------
// Pages that are not what the parser expects
// ---------------------------------------------------------------------------

#[test]
fn garbage_and_truncated_pages_are_parser_outdated() {
    let garbage = "<html><head><title>Parfumo</title></head><body><p>maintenance</p></body></html>";
    assert_outdated(parse::parse_forum_home(garbage));
    assert_outdated(parse::parse_forum_category(garbage, 1));
    assert_outdated(parse::parse_forum_thread(garbage, 1));
    assert_outdated(parse::parse_forum_search(garbage, 1));
    assert_outdated(parse::parse_communities(garbage, 1));
    assert_outdated(parse::parse_community(garbage, 1));
    assert_outdated(parse::parse_community_post(garbage));

    assert_outdated(parse::parse_forum_home(&truncated("forum_home.html", 1)));
    assert_outdated(parse::parse_forum_category(&truncated("forum_category.html", 1), 1));
    assert_outdated(parse::parse_forum_thread(&truncated("forum_topic.html", 1), 1));
    assert_outdated(parse::parse_forum_search(&truncated("forum_search.html", 1), 1));
    assert_outdated(parse::parse_communities(&truncated("communities.html", 1), 1));
    assert_outdated(parse::parse_community(&truncated("community.html", 1), 1));
    assert_outdated(parse::parse_community_post(&truncated("community_post.html", 1)));

    // Cut in the middle of the list: the items read so far are not passed off as a short page.
    assert_outdated(parse::parse_forum_home(&truncated("forum_home.html", 2)));
    assert_outdated(parse::parse_forum_category(&truncated("forum_category.html", 2), 1));
    assert_outdated(parse::parse_forum_thread(&truncated("forum_topic.html", 2), 1));
    assert_outdated(parse::parse_forum_search(&truncated("forum_search.html", 2), 1));
    assert_outdated(parse::parse_communities(&truncated("communities.html", 2), 1));
    assert_outdated(parse::parse_community(&truncated("community.html", 2), 1));
}

#[test]
fn a_page_of_another_kind_is_parser_outdated() {
    assert_outdated(parse::parse_forum_home(&fixture("forum_category.html")));
    assert_outdated(parse::parse_forum_category(&fixture("forum_home.html"), 1));
    assert_outdated(parse::parse_forum_category(&fixture("forum_topic.html"), 1));
    assert_outdated(parse::parse_forum_thread(&fixture("forum_category.html"), 1));
    assert_outdated(parse::parse_forum_search(&fixture("forum_home.html"), 1));
    assert_outdated(parse::parse_communities(&fixture("community.html"), 1));
    assert_outdated(parse::parse_community(&fixture("communities.html"), 1));
    assert_outdated(parse::parse_community(&fixture("community_post.html"), 1));
    assert_outdated(parse::parse_community_post(&fixture("community.html")));
}

#[test]
fn the_404_page_is_not_found() {
    let html = fixture("notfound.html");
    assert!(matches!(parse::parse_forum_category(&html, 1), Err(ClientError::NotFound(_))));
    assert!(matches!(parse::parse_forum_thread(&html, 9), Err(ClientError::NotFound(_))));
    assert!(matches!(parse::parse_community(&html, 1), Err(ClientError::NotFound(_))));
    assert!(matches!(parse::parse_community_post(&html), Err(ClientError::NotFound(_))));
}

// ---------------------------------------------------------------------------
// Client routing against a local mock (which path each method requests)
// ---------------------------------------------------------------------------

type Seen = Arc<Mutex<Vec<String>>>;

async fn serve(State(seen): State<Seen>, uri: Uri) -> impl IntoResponse {
    let target = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    seen.lock().unwrap().push(target);
    let path = uri.path();
    let name = if path == "/forums" {
        "forum_home.html"
    } else if path.starts_with("/forums/post/") || path.ends_with("/2") {
        "forum_topic_page2.html"
    } else if path.starts_with("/forums/topic/") {
        "forum_topic.html"
    } else if path.starts_with("/forums/") {
        "forum_category.html"
    } else if path == "/s_forum_x.php" {
        if uri.query().unwrap_or_default().contains("zzz") { "forum_search_empty.html" } else { "forum_search.html" }
    } else if path == "/communities/discover" {
        "communities.html"
    } else if path.contains("/post/") {
        "community_post.html"
    } else if path.starts_with("/communities/") {
        "community.html"
    } else {
        "notfound.html"
    };
    ([("content-type", "text/html; charset=utf-8")], fixture(name))
}

async fn start() -> (ParfumoClient, Seen, String) {
    let seen: Seen = Arc::default();
    let app = Router::new().fallback(serve).with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cfg = ClientConfig { base_url: base.clone(), min_interval: Duration::ZERO, jitter: Duration::ZERO, ..ClientConfig::default() };
    (ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap(), seen, base)
}

fn take(seen: &Seen) -> Vec<String> {
    std::mem::take(&mut *seen.lock().unwrap())
}

#[tokio::test]
async fn forum_methods_request_the_measured_routes() {
    let (client, seen, base) = start().await;

    assert_eq!(client.forum_categories().await.unwrap().len(), 8);
    assert_eq!(take(&seen), ["/forums"]);

    assert_eq!(client.forum_topics("perfumes-brands", 1).await.unwrap().category.forum_id, Some(7));
    client.forum_topics("/forums/perfumes-brands", 3).await.unwrap();
    client.forum_topics(&format!("{base}/forums/perfumes-brands/4"), 0).await.unwrap();
    assert_eq!(take(&seen), ["/forums/perfumes-brands", "/forums/perfumes-brands/3", "/forums/perfumes-brands/4"]);

    assert_eq!(client.forum_thread(TOPIC, 1).await.unwrap().posts.page, 1);
    assert_eq!(client.forum_thread(&format!("forums/topic/{TOPIC}"), 2).await.unwrap().posts.page, 2);
    client.forum_thread(&format!("{base}/forums/topic/{TOPIC}/2?x=1#213001"), 1).await.unwrap();
    assert_eq!(take(&seen), [format!("/forums/topic/{TOPIC}"), format!("/forums/topic/{TOPIC}/2"), format!("/forums/topic/{TOPIC}/2")]);

    // A post URL shows the page holding the post; another page is then read by slug.
    let thread = client.forum_thread(&format!("{base}/forums/post/213001#213001"), 1).await.unwrap();
    assert_eq!(thread.posts.page, 2);
    assert!(thread.posts.items.iter().any(|p| p.id == 213001));
    assert_eq!(take(&seen), ["/forums/post/213001"]);
    client.forum_thread("/forums/post/213001", 2).await.unwrap();
    assert_eq!(take(&seen), ["/forums/post/213001"]);
    client.forum_thread("post/213001", 3).await.unwrap();
    assert_eq!(take(&seen), ["/forums/post/213001".to_string(), format!("/forums/topic/{TOPIC}/3")]);

    assert_eq!(client.forum_search(" grey vetiver ", 1).await.unwrap().items.len(), 20);
    client.forum_search("vétiver & co", 2).await.unwrap();
    assert!(client.forum_search("zzz", 1).await.unwrap().items.is_empty());
    assert_eq!(
        take(&seen),
        ["/s_forum_x.php?filter=grey+vetiver", "/s_forum_x.php?filter=v%C3%A9tiver+%26+co&current_page=2", "/s_forum_x.php?filter=zzz",]
    );
}

#[tokio::test]
async fn community_methods_request_the_measured_routes() {
    let (client, seen, base) = start().await;

    // The mock repeats the first directory page: the walk stops as soon as a page brings nothing new.
    let all = client.communities().await.unwrap();
    assert_eq!(all.len(), 12);
    assert_eq!(take(&seen), ["/communities/discover", "/communities/discover?page=2"]);
    assert_eq!(client.communities_page(3).await.unwrap().total_items, Some(105));
    assert_eq!(take(&seen), ["/communities/discover?page=3"]);

    assert_eq!(client.community("scent-of-the-day", 1).await.unwrap().community.group_id, Some(74));
    assert_eq!(client.community(&format!("{base}/communities/scent-of-the-day"), 2).await.unwrap().posts.page, 2);
    assert_eq!(take(&seen), ["/communities/scent-of-the-day", "/communities/scent-of-the-day?page=2"]);

    let wanted = format!("/communities/the-slightly-haunted-dollhouse/post/{POST_SLUG}");
    assert_eq!(client.community_post("the-slightly-haunted-dollhouse", POST_SLUG).await.unwrap().post.id, Some(3592));
    client.community_post("", &format!("{base}{wanted}#reply")).await.unwrap();
    client.community_post("ignored", &wanted).await.unwrap();
    client.community_post("/communities/the-slightly-haunted-dollhouse", &format!("post/{POST_SLUG}")).await.unwrap();
    assert_eq!(take(&seen), [wanted.clone(), wanted.clone(), wanted.clone(), wanted]);
}

#[tokio::test]
async fn bad_references_are_refused_without_any_request() {
    let (client, seen, base) = start().await;
    assert_invalid(client.forum_topics("https://evil.example/forums/perfumes-brands", 1).await);
    assert_invalid(client.forum_topics("topic", 1).await);
    assert_invalid(client.forum_topics("/Perfumes/Dior", 1).await);
    assert_invalid(client.forum_topics("", 1).await);
    assert_invalid(client.forum_thread("https://evil.example/forums/topic/x", 1).await);
    assert_invalid(client.forum_thread(&format!("{base}/forums/perfumes-brands"), 1).await);
    assert_invalid(client.forum_thread("/forums/post/abc", 1).await);
    assert_invalid(client.forum_thread("some topic?", 1).await);
    assert_invalid(client.forum_thread("../etc", 1).await);
    assert_invalid(client.forum_search("   ", 1).await);
    assert_invalid(client.forum_search(&"x".repeat(161), 1).await);
    assert_invalid(client.community("https://evil.example/communities/scent-of-the-day", 1).await);
    assert_invalid(client.community("discover", 1).await);
    assert_invalid(client.community("/forums/perfumes-brands", 1).await);
    assert_invalid(client.community_post("scent-of-the-day", "https://evil.example/communities/a/post/b").await);
    assert_invalid(client.community_post("", "floral-and-spicy").await);
    assert_invalid(client.community_post("scent-of-the-day", &format!("{base}/communities/scent-of-the-day")).await);
    assert!(take(&seen).is_empty());
}

/// Seen live at jalon 4: a post made of a title and a perfume has no text block at all.
#[test]
fn a_community_post_without_text_is_not_a_parser_failure() {
    let html = std::fs::read_to_string(format!("{}/tests/fixtures/community_post.html", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let with_text = parfumo_client::parse::parse_community_post(&html).unwrap();
    assert!(!with_text.post.text.is_empty());
    let without = html.replace("g-post-detail-body", "g-post-detail-other");
    assert_ne!(without, html);
    let page = parfumo_client::parse::parse_community_post(&without).unwrap();
    assert_eq!((page.post.text.as_str(), page.post.slug == with_text.post.slug, page.post.title == with_text.post.title), ("", true, true));
}
