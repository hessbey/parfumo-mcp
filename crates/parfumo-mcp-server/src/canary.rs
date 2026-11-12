//! Canary (cahier des charges, section 9, "Maintenance"): once a day, read every kind of
//! page and route the tools depend on, on the live site, and report whatever no longer has
//! the structure the parsers expect.
//!
//! The canary only reads. It sends no write, not even a dry run, and uses clients without
//! cache, so that it sees the site as it is now. It goes through the same rate limiter as
//! the tools (one request per second), and stops at once on a captcha or a rate limit.

use std::{future::Future, time::Instant};

use parfumo_client::{BlogFeed, ClientError, ParfumoClient, RankingKind, ReviewSort, SearchQuery, SettingsPage, SimilarOptions, SoukKind, TextKind};
use serde::{Deserialize, Serialize};

use parfumo_mcp_core::db;

use crate::web::ServerState;

/// A perfume that has existed for years, with reviews, statements and similar perfumes.
const PERFUME: &str = "Dior/Sauvage_Eau_de_Toilette";
const BRAND: &str = "Dior";
const NOTE: &str = "Bergamot";
const SEARCH_TEXT: &str = "sauvage";
const FORUM_CATEGORY: &str = "perfume-discussions";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The route answered and the parser found what it expects.
    Ok,
    /// The structure changed: a parser or a route must be updated.
    Changed,
    /// No verdict: Parfumo was unreachable, refused, or the account session is gone.
    Failed,
    /// Not run (an earlier check it depends on did not pass, or the run was stopped).
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    /// `public` or `account`.
    pub area: String,
    pub name: String,
    /// The client function behind the check, to find the parser in `parfumo-client`.
    pub function: String,
    pub outcome: Outcome,
    pub detail: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Unix time of the start of the run.
    pub started_at: i64,
    pub base_url: String,
    /// Server user whose linked Parfumo account was read, if any.
    pub account_user: Option<String>,
    /// `ok`, `changed` (at least one structure changed) or `inconclusive` (nothing
    /// changed for sure, but some checks could not run).
    pub status: String,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn count(&self, outcome: Outcome) -> usize {
        self.checks.iter().filter(|c| c.outcome == outcome).count()
    }

    pub fn summary(&self) -> String {
        format!(
            "canary {}: {} ok, {} changed, {} failed, {} skipped ({})",
            self.status,
            self.count(Outcome::Ok),
            self.count(Outcome::Changed),
            self.count(Outcome::Failed),
            self.count(Outcome::Skipped),
            match &self.account_user {
                Some(u) => format!("public pages and the Parfumo account of `{u}`"),
                None => "public pages only".to_string(),
            }
        )
    }

    /// Human-readable report: the summary, then one line per check.
    pub fn text(&self) -> String {
        let mut out = format!("{}\n{} · {}\n", self.summary(), db_time(self.started_at), self.base_url);
        for c in &self.checks {
            let mark = match c.outcome {
                Outcome::Ok => "ok     ",
                Outcome::Changed => "CHANGED",
                Outcome::Failed => "FAILED ",
                Outcome::Skipped => "skipped",
            };
            out.push_str(&format!("{mark}  {:<7}  {:<28}  {:>6} ms  {}  [{}]\n", c.area, c.name, c.duration_ms, c.detail, c.function));
        }
        out
    }

    /// The checks that need attention, for an alert.
    pub fn problems(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| matches!(c.outcome, Outcome::Changed | Outcome::Failed)).collect()
    }
}

pub use parfumo_mcp_core::audit::db_time;

/// Collects the checks of one run and decides what to skip.
#[derive(Default)]
struct Runner {
    checks: Vec<Check>,
    /// Set by a captcha or a rate limit: nothing more is sent.
    stop: Option<String>,
    /// Set when the account session is unusable: the account checks are skipped.
    account_stop: Option<String>,
}

impl Runner {
    fn push(&mut self, area: &str, name: &str, function: &str, outcome: Outcome, detail: String, started: Option<Instant>) {
        let duration_ms = started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        self.checks.push(Check { area: area.into(), name: name.into(), function: function.into(), outcome, detail, duration_ms });
    }

    fn skip(&mut self, area: &str, name: &str, function: &str, why: &str) {
        self.push(area, name, function, Outcome::Skipped, why.to_string(), None);
    }

    /// Runs one read. `expect` looks at the parsed value: `Ok(what was found)` or
    /// `Err(what is missing)`, the latter meaning the structure changed without the parser
    /// noticing. Returns the value when the check passed, for the checks that build on it.
    async fn check<T>(
        &mut self,
        area: &str,
        name: &str,
        function: &str,
        read: impl Future<Output = Result<T, ClientError>>,
        expect: impl FnOnce(&T) -> Result<String, String>,
    ) -> Option<T> {
        let blocked = self.stop.clone().or_else(|| if area == "account" { self.account_stop.clone() } else { None });
        if let Some(why) = blocked {
            self.skip(area, name, function, &why);
            return None;
        }
        let started = Instant::now();
        match read.await {
            Ok(value) => match expect(&value) {
                Ok(found) => {
                    self.push(area, name, function, Outcome::Ok, found, Some(started));
                    Some(value)
                }
                Err(missing) => {
                    self.push(area, name, function, Outcome::Changed, format!("parsed, but {missing}"), Some(started));
                    None
                }
            },
            Err(e) => {
                let outcome = match &e {
                    // A fixed, long-lived target that vanished is a changed route.
                    ClientError::ParserOutdated(_) | ClientError::NotFound(_) | ClientError::InvalidInput(_) => Outcome::Changed,
                    ClientError::CaptchaRequired | ClientError::RateLimited { .. } => {
                        self.stop = Some(format!("run stopped after `{name}`: {e}"));
                        Outcome::Failed
                    }
                    ClientError::LoginRequired | ClientError::InvalidCredentials if area == "account" => {
                        self.account_stop = Some(format!("the account session was refused at `{name}`"));
                        Outcome::Failed
                    }
                    // A public page that now asks for a login is a changed route.
                    ClientError::LoginRequired | ClientError::InvalidCredentials => Outcome::Changed,
                    ClientError::Upstream(_) => Outcome::Failed,
                };
                // `ClientError` never carries a secret (invariant of parfumo-client).
                self.push(area, name, function, outcome, e.to_string(), Some(started));
                None
            }
        }
    }

    fn status(&self) -> &'static str {
        if self.checks.iter().any(|c| c.outcome == Outcome::Changed) {
            "changed"
        } else if self.checks.iter().any(|c| matches!(c.outcome, Outcome::Failed | Outcome::Skipped)) {
            "inconclusive"
        } else {
            "ok"
        }
    }
}

fn some(n: usize, what: &str) -> Result<String, String> {
    if n == 0 { Err(format!("no {what} found")) } else { Ok(format!("{n} {what}")) }
}

/// An empty list is a legitimate state of an account: only the parser's own checks apply.
fn any(n: usize, what: &str) -> Result<String, String> {
    Ok(format!("{n} {what}"))
}

fn filled(value: &str, what: &str) -> Result<(), String> {
    if value.trim().is_empty() { Err(format!("the {what} is empty")) } else { Ok(()) }
}

/// Reads of the public site, as the anonymous client of the tools does.
async fn public_checks(r: &mut Runner, c: &ParfumoClient) {
    const A: &str = "public";
    let query = SearchQuery { text: Some(SEARCH_TEXT.into()), ..SearchQuery::default() };
    r.check(A, "search", "search_perfumes", c.search_perfumes(&query, 1), |l| {
        if l.items.iter().any(|p| p.name.trim().is_empty() || p.url.is_empty()) {
            return Err("a result has no name or no link".into());
        }
        some(l.items.len(), "results")
    })
    .await;

    let perfume = r
        .check(A, "perfume", "get_perfume", c.get_perfume(PERFUME), |p| {
            filled(&p.name, "perfume name")?;
            filled(&p.brand.name, "brand")?;
            if p.p_id.is_none() {
                return Err("the numeric id is missing".into());
            }
            let notes = p.notes.top.len() + p.notes.heart.len() + p.notes.base.len() + p.notes.flat.len();
            if notes == 0 || p.ratings.is_empty() || p.perfumers.is_empty() {
                return Err(format!("{notes} notes, {} ratings, {} perfumers", p.ratings.len(), p.perfumers.len()));
            }
            if !p.warnings.is_empty() {
                return Err(format!("the parser warns: {}", p.warnings.join("; ")));
            }
            Ok(format!("{notes} notes, {} ratings, {} classification groups", p.ratings.len(), p.classification.len()))
        })
        .await;
    r.check(A, "perfume reviews", "get_perfume_reviews", c.get_perfume_reviews(PERFUME, ReviewSort::Relevance, 0, 3), |s| {
        if s.items.iter().any(|v| v.author.is_empty() || v.text.trim().is_empty()) {
            return Err("a review has no author or no text".into());
        }
        some(s.items.len(), "reviews")
    })
    .await;
    r.check(A, "perfume statements", "get_perfume_statements", c.get_perfume_statements(PERFUME, ReviewSort::Relevance, 0, 3), |s| {
        if s.items.iter().any(|v| v.author.is_empty() || v.text.trim().is_empty()) {
            return Err("a statement has no author or no text".into());
        }
        some(s.items.len(), "statements")
    })
    .await;
    let similar = SimilarOptions { resolve_urls: 1, include_layering: false, include_dupes: true };
    r.check(A, "similar perfumes", "get_similar_perfumes_with", c.get_similar_perfumes_with(PERFUME, similar), |s| {
        if !s.warnings.is_empty() {
            return Err(format!("the parser warns: {}", s.warnings.join("; ")));
        }
        if s.similar.first().is_some_and(|p| p.url.is_none()) {
            return Err("the first similar perfume has no link".into());
        }
        some(s.similar.len(), "similar perfumes")
    })
    .await;
    r.check(A, "brand", "get_brand", c.get_brand(BRAND, 1), |b| {
        filled(&b.brand.name, "brand name")?;
        some(b.perfumes.items.len(), "perfumes")
    })
    .await;
    match perfume.as_ref().and_then(|p| p.perfumers.first()) {
        Some(perfumer) => {
            r.check(A, "perfumer", "get_perfumer", c.get_perfumer(&perfumer.id, 1), |b| {
                filled(&b.perfumer.name, "perfumer name")?;
                some(b.perfumes.items.len(), "perfumes")
            })
            .await;
        }
        None => r.skip(A, "perfumer", "get_perfumer", "needs the perfumer of the `perfume` check"),
    }
    r.check(A, "note", "get_note", c.get_note(NOTE, 1), |b| {
        filled(&b.note.name, "note name")?;
        some(b.perfumes.items.len(), "perfumes")
    })
    .await;
    r.check(A, "ranking by gender", "get_rankings", c.get_rankings(&RankingKind::TopMen, 1), |l| some(l.items.len(), "perfumes")).await;
    r.check(A, "ranking trends", "get_rankings", c.get_rankings(&RankingKind::Trends, 1), |l| some(l.items.len(), "perfumes")).await;
    r.check(A, "page text", "fetch_page", c.fetch_page("/Perfumes"), |p| {
        filled(&p.text, "page text")?;
        some(p.links.len(), "links")
    })
    .await;

    // Forum.
    let categories = r.check(A, "forum categories", "forum_categories", c.forum_categories(), |l| some(l.len(), "categories")).await;
    let category = categories.as_ref().and_then(|l| l.iter().find(|c| c.slug == FORUM_CATEGORY).or(l.first())).map(|c| c.slug.clone());
    let topics = match &category {
        Some(slug) => r.check(A, "forum topics", "forum_topics", c.forum_topics(slug, 1), |p| some(p.topics.items.len(), "topics")).await,
        None => {
            r.skip(A, "forum topics", "forum_topics", "needs a category of the `forum categories` check");
            None
        }
    };
    match topics.as_ref().and_then(|p| p.topics.items.iter().find(|t| !t.pinned).or(p.topics.items.first())) {
        Some(topic) => {
            r.check(A, "forum thread", "forum_thread", c.forum_thread(&topic.slug, 1), |t| {
                filled(&t.topic.title, "topic title")?;
                if t.posts.items.iter().any(|p| p.author.is_empty()) {
                    return Err("a post has no author".into());
                }
                some(t.posts.items.len(), "posts")
            })
            .await;
        }
        None => r.skip(A, "forum thread", "forum_thread", "needs a topic of the `forum topics` check"),
    }
    r.check(A, "forum search", "forum_search", c.forum_search("vanilla", 1), |l| some(l.items.len(), "hits")).await;

    // Communities.
    let communities = r.check(A, "communities", "communities_page", c.communities_page(1), |l| some(l.items.len(), "communities")).await;
    let community = match communities.as_ref().and_then(|l| l.items.first()) {
        Some(group) => {
            r.check(A, "community", "community", c.community(&group.slug, 1), |p| {
                filled(&p.community.name, "community name")?;
                any(p.posts.items.len(), "posts")
            })
            .await
        }
        None => {
            r.skip(A, "community", "community", "needs a community of the `communities` check");
            None
        }
    };
    match community.as_ref().and_then(|p| p.posts.items.first()) {
        Some(post) => {
            r.check(A, "community post", "community_post", c.community_post(&post.community_slug, &post.slug), |p| {
                // The parser always warns that comments are reserved to members: not a change.
                filled(&p.post.slug, "post slug")?;
                filled(&p.community.name, "community name")?;
                Ok(format!("post `{}`, {} characters", p.post.slug, p.post.text.chars().count()))
            })
            .await;
        }
        None => r.skip(A, "community post", "community_post", "the community read has no post"),
    }

    // Blogs.
    let blogs = r
        .check(A, "blog feed", "blog_feed", c.blog_feed(BlogFeed::Latest, 1), |l| {
            if l.items.iter().any(|b| b.title.is_empty() || b.author.is_empty()) {
                return Err("an article has no title or no author".into());
            }
            some(l.items.len(), "articles")
        })
        .await;
    let entry = blogs.as_ref().and_then(|l| l.items.first());
    match entry {
        Some(e) => {
            r.check(A, "blog article", "blog_article", c.blog_article(&e.url), |a| {
                filled(&a.title, "article title")?;
                filled(&a.text, "article text")?;
                Ok(format!("{} characters, {} comments shown", a.text.chars().count(), a.comment_list.len()))
            })
            .await;
        }
        None => r.skip(A, "blog article", "blog_article", "needs an article of the `blog feed` check"),
    }

    // Souk.
    let offers = r
        .check(A, "souk search", "souk_search", c.souk_search(SoukKind::Offers, Some(BRAND), 1), |l| {
            if l.items.iter().any(|i| i.perfume_name.is_empty() || i.author.is_empty()) {
                return Err("an entry has no perfume or no author".into());
            }
            some(l.items.len(), "offers")
        })
        .await;
    r.check(A, "souk latest", "souk_search", c.souk_search(SoukKind::Requests, None, 1), |l| some(l.items.len(), "requests")).await;
    match offers.as_ref().and_then(|l| l.items.first()) {
        Some(item) => {
            r.check(A, "souk entry", "souk_item", c.souk_item(&item.url), |i| {
                filled(&i.perfume_name, "perfume name")?;
                filled(&i.author, "author")?;
                // Lists carry none of these: an entry page without any has lost its table.
                let fields = [&i.condition, &i.size, &i.price, &i.shipping, &i.swap_for, &i.note, &i.item_type, &i.trade]
                    .iter()
                    .filter(|f| f.is_some())
                    .count()
                    + i.details.len();
                some(fields, "detail fields")
            })
            .await;
        }
        None => r.skip(A, "souk entry", "souk_item", "needs an entry of the `souk search` check"),
    }

    // A member: the author of the latest blog article (public profile, lists, wall, texts).
    let Some(nick) = entry.map(|e| e.author.clone()) else {
        for (name, function) in [
            ("member profile", "member_profile"),
            ("member wall", "user_wall"),
            ("member reviews", "member_texts_page"),
            ("member collection", "member_collection"),
        ] {
            r.skip(A, name, function, "needs a member of the `blog feed` check");
        }
        return;
    };
    let profile = r
        .check(A, "member profile", "member_profile", c.member_profile(&nick, false), |p| {
            filled(&p.nick, "nickname")?;
            some(p.counters.len(), "counters")
        })
        .await;
    r.check(A, "member wall", "user_wall", c.user_wall(&nick, 1), |w| {
        filled(&w.owner, "wall owner")?;
        any(w.entries.items.len(), "entries")
    })
    .await;
    r.check(A, "member reviews", "member_texts_page", c.member_texts_page(&nick, TextKind::Review, 1), |l| any(l.items.len(), "reviews")).await;
    match profile.as_ref().and_then(|p| p.collections.iter().find(|l| l.count.unwrap_or(0) > 0)) {
        Some(list) => {
            r.check(A, "member collection", "member_collection", c.member_collection(&nick, &list.id), |(_, items)| some(items.len(), "perfumes"))
                .await;
        }
        None => r.skip(A, "member collection", "member_collection", "this member shows no list with perfumes"),
    }
}

/// Reads of the linked (test) account. Lists may be empty: the parsers still check the
/// structure of the page around them.
async fn account_checks(r: &mut Runner, c: &ParfumoClient, p_id: Option<u64>) {
    const A: &str = "account";
    let nick = r.check(A, "own nickname", "own_nick", c.own_nick(), |n| filled(n, "nickname").map(|()| "found".to_string())).await;
    r.check(A, "collection lists", "shelf_default", c.shelf_default(), |s| {
        if s.lists.len() < 7 {
            return Err(format!("{} lists instead of the 7 built-in ones", s.lists.len()));
        }
        Ok(format!("{} lists, {} perfumes in the first", s.lists.len(), s.items.len()))
    })
    .await;
    r.check(A, "collection wish list", "shelf", c.shelf("want"), |s| any(s.items.len(), "perfumes")).await;
    r.check(A, "collection csv", "collection_csv", c.collection_csv(1), |csv| {
        if csv.contains("Perfume") && csv.contains("Brand") {
            Ok(format!("{} lines", csv.lines().count()))
        } else {
            Err("the header line has no `Perfume` and `Brand` columns".into())
        }
    })
    .await;
    r.check(A, "wears overview", "wears_overview", c.wears_overview(), |w| any(w.len(), "wears")).await;
    let (year, month, _) = parfumo_client::parse::civil_from_days(db::now().div_euclid(86_400));
    r.check(A, "wears of the month", "wears_of_month", c.wears_of_month(year, month), |w| any(w.len(), "wears")).await;
    r.check(A, "my ratings", "my_ratings_page", c.my_ratings_page("scent", "d_desc", 1), |l| any(l.items.len(), "ratings")).await;
    r.check(A, "my reviews", "my_texts_page", c.my_texts_page(TextKind::Review, 1), |l| any(l.items.len(), "reviews")).await;
    r.check(A, "my statements", "my_texts_page", c.my_texts_page(TextKind::Statement, 1), |l| any(l.items.len(), "statements")).await;
    match p_id {
        Some(p_id) => {
            r.check(A, "rating panel", "my_ratings_of", c.my_ratings_of(p_id), |m| {
                Ok(if m.is_empty() { "not rated".to_string() } else { "rated".to_string() })
            })
            .await;
            r.check(A, "classification panel", "my_classification_of", c.my_classification_of(p_id), |l| some(l.len(), "choices")).await;
            r.check(A, "my review of a perfume", "my_text_on", c.my_text_on(TextKind::Review, p_id), |d| any(d.text.chars().count(), "characters"))
                .await;
            r.check(A, "my statement on a perfume", "my_text_on", c.my_text_on(TextKind::Statement, p_id), |d| {
                any(d.text.chars().count(), "characters")
            })
            .await;
        }
        None => {
            for (name, function) in [
                ("rating panel", "my_ratings_of"),
                ("classification panel", "my_classification_of"),
                ("my review of a perfume", "my_text_on"),
                ("my statement on a perfume", "my_text_on"),
            ] {
                r.skip(A, name, function, "needs the numeric id of the `perfume` check");
            }
        }
    }
    r.check(A, "mailbox", "mailbox", c.mailbox(0), |m| any(m.items.len(), "conversations")).await;
    r.check(A, "notifications", "notifications", c.notifications(), |n| any(n.len(), "notifications")).await;
    r.check(A, "following", "following", c.following(), |f| any(f.members.len(), "members followed")).await;
    for page in SettingsPage::ALL {
        r.check(A, &format!("settings {page:?}"), "settings", c.settings(page), |s| {
            if s.iter().any(|x| x.field.is_empty()) {
                return Err("a setting has no field name".into());
            }
            some(s.len(), "settings")
        })
        .await;
    }
    r.check(A, "fragrance preferences", "fragrance_preferences", c.fragrance_preferences(), |_| Ok("form read".to_string())).await;
    match nick {
        Some(nick) => {
            r.check(A, "own profile", "member_profile", c.member_profile(&nick, false), |p| {
                filled(&p.nick, "nickname")?;
                // An account with empty lists shows none on its profile.
                any(p.collections.len(), "lists shown")
            })
            .await;
        }
        None => r.skip(A, "own profile", "member_profile", "needs the `own nickname` check"),
    }
}

/// One full run. `user`: the server user whose linked Parfumo account is read after the
/// public pages (`None`: public pages only). The report is stored and logged.
pub async fn run(state: &ServerState, user: Option<&db::User>) -> Report {
    let started_at = db::now();
    let mut r = Runner::default();
    // Limiter slot of the canary user, or of nobody (0): the global budget applies anyway.
    let slot = user.map(|u| u.id).unwrap_or(0);
    let mut p_id = None;
    match state.core.sessions.uncached_public_client(slot) {
        Ok(client) => {
            public_checks(&mut r, &client).await;
            if user.is_some() && r.stop.is_none() {
                // Cheap when the perfume check passed: the page is asked for again only here.
                p_id = client.perfume_numeric_id(PERFUME).await.ok().map(|(id, _)| id);
            }
        }
        Err(e) => r.push("public", "client", "ParfumoClient::new", Outcome::Failed, e.to_string(), None),
    }
    if let Some(user) = user {
        if let Some(why) = r.stop.clone() {
            r.skip("account", "account session", "account_client", &why);
        } else {
            match state.core.sessions.account_client(user.id).await {
                Ok(client) => account_checks(&mut r, &client, p_id).await,
                Err(e) => r.push("account", "account session", "account_client", Outcome::Failed, e.to_string(), None),
            }
        }
    }
    let report = Report {
        started_at,
        base_url: state.core.cfg.parfumo_base_url.clone(),
        account_user: user.map(|u| u.name.clone()),
        status: r.status().to_string(),
        checks: r.checks,
    };
    record(state, &report);
    report
}

fn record(state: &ServerState, report: &Report) {
    let json = serde_json::to_string(report).unwrap_or_default();
    if let Err(e) = state.core.db.canary_put(&report.status, &json) {
        tracing::error!(error = %e, "canary report could not be stored");
    }
    for c in report.problems() {
        tracing::error!(area = %c.area, check = %c.name, function = %c.function, outcome = ?c.outcome, detail = %c.detail, "canary check needs attention");
    }
    if report.status == "ok" {
        tracing::info!(checks = report.checks.len(), "{}", report.summary());
    } else {
        tracing::error!(status = %report.status, "{}", report.summary());
    }
}

/// The last stored report, if any.
pub fn last(db: &db::Db) -> Option<Report> {
    db.canary_last().ok().flatten().and_then(|row| serde_json::from_str(&row.report).ok())
}

/// Posts an alert to the operator's webhook. The body is JSON with the same sentence
/// under `text` and `content`, which Slack-style and Discord-style webhooks both accept,
/// plus the checks that need attention. Never contains account data.
pub async fn notify(webhook: &str, report: &Report) -> Result<(), String> {
    let mut text = report.summary();
    for c in report.problems().iter().take(15) {
        text.push_str(&format!("\n- {} / {} ({}): {}", c.area, c.name, c.function, c.detail));
    }
    let body = serde_json::json!({
        "text": text,
        "content": text,
        "status": report.status,
        "started_at": report.started_at,
        "problems": report.problems(),
    });
    let http = reqwest::Client::builder()
        .use_preconfigured_tls(parfumo_client::tls_config())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http
        .post(webhook)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("webhook unreachable: {}", e.without_url()))?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("webhook answered HTTP {}", resp.status().as_u16())) }
}

/// Runs the canary, then alerts when the result is not `ok`, or when it is `ok` again
/// after a run that was not.
pub async fn run_and_notify(state: &ServerState) -> Report {
    let user = match &state.http.canary_user {
        Some(name) => match state.core.db.user_by_name(name) {
            Ok(Some(u)) => Some(u),
            _ => {
                tracing::error!(user = %name, "PARFUMO_MCP_CANARY_USER names no user of this server: the canary reads public pages only");
                None
            }
        },
        None => None,
    };
    let before = state.core.db.canary_last().ok().flatten().map(|r| r.status);
    let report = run(state, user.as_ref()).await;
    let recovered = report.status == "ok" && before.as_deref().is_some_and(|s| s != "ok");
    if let Some(webhook) = state.http.canary_webhook.as_deref().filter(|_| report.status != "ok" || recovered)
        && let Err(e) = notify(webhook, &report).await
    {
        tracing::error!(error = %e, "canary alert could not be delivered");
    }
    report
}

/// Seconds from `now` to the next `hour`:00 UTC (never 0: a run at the hour waits a day).
pub fn secs_until_hour(now: i64, hour: u8) -> u64 {
    let today = now.rem_euclid(86_400);
    let target = i64::from(hour) * 3600;
    let wait = (target - today).rem_euclid(86_400);
    if wait == 0 { 86_400 } else { wait as u64 }
}

/// Background task of the server: one run a day at the configured hour (UTC).
pub async fn daily(state: ServerState, hour: u8) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(secs_until_hour(db::now(), hour))).await;
        run_and_notify(&state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outcomes_follow_the_error_and_the_expectation() {
        let mut r = Runner::default();
        let ok = r.check("public", "a", "f", async { Ok::<_, ClientError>(3) }, |n| Ok(format!("{n} items"))).await;
        assert_eq!(ok, Some(3));
        let none = r.check("public", "b", "f", async { Ok::<_, ClientError>(0) }, |_| Err("no item found".into())).await;
        assert_eq!(none, None);
        r.check("public", "c", "f", async { Err::<u32, _>(ClientError::ParserOutdated("rating block".into())) }, |_| Ok(String::new())).await;
        r.check("public", "d", "f", async { Err::<u32, _>(ClientError::NotFound("page".into())) }, |_| Ok(String::new())).await;
        r.check("public", "e", "f", async { Err::<u32, _>(ClientError::Upstream("HTTP 503".into())) }, |_| Ok(String::new())).await;
        let outcomes: Vec<Outcome> = r.checks.iter().map(|c| c.outcome).collect();
        assert_eq!(outcomes, [Outcome::Ok, Outcome::Changed, Outcome::Changed, Outcome::Changed, Outcome::Failed]);
        assert!(r.checks[1].detail.contains("no item found"));
        assert!(r.checks[2].detail.contains("rating block"));
        assert_eq!(r.status(), "changed");
    }

    #[tokio::test]
    async fn a_captcha_or_a_rate_limit_stops_the_run_at_once() {
        for stopper in [ClientError::CaptchaRequired, ClientError::RateLimited { retry_after_secs: Some(60) }] {
            let mut r = Runner::default();
            r.check("public", "first", "f", async { Err::<u32, _>(stopper.clone()) }, |_| Ok(String::new())).await;
            let sent = std::cell::Cell::new(false);
            r.check(
                "account",
                "second",
                "f",
                async {
                    sent.set(true);
                    Ok::<_, ClientError>(1)
                },
                |_| Ok(String::new()),
            )
            .await;
            assert!(!sent.get(), "nothing is sent after {stopper:?}");
            assert_eq!(r.checks.iter().map(|c| c.outcome).collect::<Vec<_>>(), [Outcome::Failed, Outcome::Skipped]);
            assert!(r.checks[1].detail.contains("run stopped after `first`"));
            assert_eq!(r.status(), "inconclusive", "a refusal is not a changed structure");
        }
    }

    #[tokio::test]
    async fn a_lost_account_session_skips_the_account_checks_only() {
        let mut r = Runner::default();
        r.check("account", "own nickname", "own_nick", async { Err::<u32, _>(ClientError::LoginRequired) }, |_| Ok(String::new())).await;
        r.check("account", "mailbox", "mailbox", async { Ok::<_, ClientError>(1) }, |_| Ok(String::new())).await;
        r.check("public", "search", "search_perfumes", async { Ok::<_, ClientError>(1) }, |_| Ok("1 result".into())).await;
        assert_eq!(r.checks.iter().map(|c| c.outcome).collect::<Vec<_>>(), [Outcome::Failed, Outcome::Skipped, Outcome::Ok]);
        assert_eq!(r.status(), "inconclusive");
        let mut clean = Runner::default();
        clean.check("public", "search", "search_perfumes", async { Ok::<_, ClientError>(1) }, |_| Ok("1 result".into())).await;
        assert_eq!(clean.status(), "ok");
    }

    #[test]
    fn the_daily_run_waits_for_the_next_occurrence_of_the_hour() {
        let midnight = 1_800_000_000 - 1_800_000_000 % 86_400;
        assert_eq!(secs_until_hour(midnight, 4), 4 * 3600);
        assert_eq!(secs_until_hour(midnight + 4 * 3600, 4), 86_400, "a run at the hour is not repeated at once");
        assert_eq!(secs_until_hour(midnight + 5 * 3600, 4), 23 * 3600);
        assert_eq!(secs_until_hour(midnight + 86_399, 0), 1);
    }

    #[test]
    fn the_report_reads_well_and_survives_storage() {
        let report = Report {
            started_at: 1_794_480_000,
            base_url: "https://www.parfumo.com".into(),
            account_user: None,
            status: "changed".into(),
            checks: vec![
                Check {
                    area: "public".into(),
                    name: "search".into(),
                    function: "search_perfumes".into(),
                    outcome: Outcome::Ok,
                    detail: "20 results".into(),
                    duration_ms: 310,
                },
                Check {
                    area: "public".into(),
                    name: "perfume".into(),
                    function: "get_perfume".into(),
                    outcome: Outcome::Changed,
                    detail: "parser outdated: rating block".into(),
                    duration_ms: 290,
                },
            ],
        };
        assert_eq!(report.summary(), "canary changed: 1 ok, 1 changed, 0 failed, 0 skipped (public pages only)");
        let text = report.text();
        assert!(text.contains("CHANGED  public   perfume") && text.contains("[get_perfume]"), "{text}");
        assert_eq!(report.problems().len(), 1);
        let db = db::Db::open_in_memory().unwrap();
        db.canary_put(&report.status, &serde_json::to_string(&report).unwrap()).unwrap();
        let back = last(&db).unwrap();
        assert_eq!((back.status.as_str(), back.checks.len(), back.checks[1].outcome), ("changed", 2, Outcome::Changed));
    }
}

#[cfg(test)]
mod area_tests {
    use super::*;

    #[tokio::test]
    async fn a_public_page_that_asks_for_a_login_is_a_change_and_does_not_stop_the_account_checks() {
        let mut r = Runner::default();
        r.check("public", "souk latest", "souk_search", async { Err::<u32, _>(ClientError::LoginRequired) }, |_| Ok(String::new())).await;
        r.check("account", "mailbox", "mailbox", async { Ok::<_, ClientError>(1) }, |_| Ok("0 conversations".into())).await;
        assert_eq!(r.checks.iter().map(|c| c.outcome).collect::<Vec<_>>(), [Outcome::Changed, Outcome::Ok]);
    }
}
