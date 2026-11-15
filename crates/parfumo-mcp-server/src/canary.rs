//! Canary (cahier des charges, section 9, "Maintenance"): once a day, read every kind of
//! page and route the tools depend on, on the live site, and report whatever no longer has
//! the structure the parsers expect.
//!
//! The canary only reads. It sends no write, not even a dry run, and sees the site as it is
//! now, like every read of the tools. It goes through the same rate limiter as
//! the tools (one request per second), and stops at once on a captcha or a rate limit.
//!
//! Parfumo is read through its web pages and through its application interface, and both
//! are checked. The run has three areas:
//! - `public`: the web pages, by a client that holds no session token of the application
//!   interface (`/app/v1`), as for a user without a linked account;
//! - `interface`: the reads the application interface serves, with the session token of the
//!   canary account, as for every user whose account is linked. Nothing is asked of the
//!   interface without a token: an account linked by the cookie of a browser has none, and
//!   the area then does not apply to it. A token the interface refuses is a failed check and
//!   is not sent again; the area goes on only if the account obtains another, once. An
//!   interface that asks to wait (its block page, a 429) stops the run, as the web site does;
//! - `account`: the pages of the canary account on the web site, by a client that holds
//!   the web session of the account and no token, so that the nickname, the rating panel
//!   and the classification panel are read on the web site too.

use std::{future::Future, time::Instant};

use parfumo_client::{
    BlogFeed, ClientError, Completed, Listing, ParfumoClient, PerfumeDetails, PerfumeSummary, RankingKind, ReviewSort, SearchQuery, SettingsPage,
    SimilarOptions, SimilarPerfumes, SoukKind, TextKind, site::routes,
};
use serde::{Deserialize, Serialize};

use parfumo_mcp_core::{
    db,
    sessions::{LinkInfo, SessionError, Sessions},
};

use crate::web::ServerState;

/// A perfume that has existed for years, with reviews, statements and similar perfumes.
const PERFUME: &str = "Dior/Sauvage_Eau_de_Toilette";
const BRAND: &str = "Dior";
const NOTE: &str = "Bergamot";
const SEARCH_TEXT: &str = "sauvage";
const FORUM_CATEGORY: &str = "perfume-discussions";
/// A release year long past, whose listing no longer moves much: the year of [`PERFUME`].
const YEAR: u16 = 2015;

/// Name of the area of the reads made through the application interface.
const INTERFACE: &str = "interface";
/// What a check of that area says when the answer does not bear the mark of the interface.
const NOT_THE_INTERFACE: &str = "the application interface did not answer this read (the web site did), or its answer changed";
/// The check that stands for the whole area when it cannot run.
const APP_SESSION: (&str, &str) = ("application session", "open_app_session");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The route answered and the parser found what it expects.
    Ok,
    /// The structure changed: a parser or a route must be updated.
    Changed,
    /// No verdict: Parfumo was unreachable, refused, or the account session is gone.
    Failed,
    /// Not run (an earlier check it depends on did not pass, or the run was stopped), or
    /// run and not judged: the account was linked again while the read was under way.
    Skipped,
    /// Not run because it does not apply to the canary account as it is: linked by the
    /// cookie of a browser, it holds no session token of the application interface; with a
    /// vote on the perfume, the interface leaves the read to the web site. Unlike a skipped
    /// check, it is the lasting state of a sound setup and takes nothing from the verdict.
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    /// `public`, `interface` or `account`.
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
        // Named only when there are some: the line is the one operators know otherwise.
        let not_applicable = match self.count(Outcome::NotApplicable) {
            0 => String::new(),
            n => format!(", {n} not applicable"),
        };
        format!(
            "canary {}: {} ok, {} changed, {} failed, {} skipped{not_applicable} ({})",
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
                Outcome::NotApplicable => "n/a    ",
            };
            out.push_str(&format!("{mark}  {:<9}  {:<28}  {:>6} ms  {}  [{}]\n", c.area, c.name, c.duration_ms, c.detail, c.function));
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
    /// Set by a captcha or a rate limit, of the web site or of the application interface:
    /// nothing more is sent.
    stop: Option<String>,
    /// Set when the account session is unusable: the account checks are skipped.
    account_stop: Option<String>,
    /// Set when nothing more is to be asked of the application interface: it refused two
    /// session tokens, or the account holds none any more.
    interface_stop: Option<String>,
    /// The check at which the application interface refused a session token, if it did.
    refused_at: Option<String>,
}

/// The client of the `interface` area and the session token it reads with: every call the
/// canary makes to the sessions about that token is here.
///
/// The client is one of its own, made like the public client of a tool call: no cookie of
/// the account, and the request slot and session token the clients of the user share. The
/// token has one owner per user: a token the interface refuses, here or to a tool call of
/// the same user, is gone for all of them at once. Before each read the sessions are asked
/// to open the session, the way a tool call does it, and after it to store what became of
/// it. Whether an account that holds none logs in for one is left to the sessions (once,
/// then not for a while); the canary itself asks nothing of the interface without a token.
struct Lent<'a> {
    sessions: &'a Sessions,
    user_id: i64,
    client: ParfumoClient,
}

impl Lent<'_> {
    /// Before a read. Returns whether the client now holds a token.
    async fn lend(&self) -> bool {
        self.sessions.open_app_session(self.user_id).await;
        self.client.has_app_session()
    }

    /// After a read that was lent a token: when the interface refused it, the account
    /// forgets it as well.
    fn give_back(&self) {
        self.sessions.store_app_session(self.user_id);
    }
}

/// A read the application interface leaves to a panel of the web site for some accounts.
struct WebPanel {
    /// The route of the panel: an error that names it was not made by the interface.
    route: &'static str,
    /// For which accounts, as the end of a sentence.
    when: &'static str,
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
        if let Some(why) = self.blocked(area) {
            self.skip(area, name, function, &why);
            return None;
        }
        let started = Instant::now();
        let answer = read.await;
        self.judge(area, name, function, answer, expect, started)
    }

    /// Why nothing more is sent for this area, if so.
    fn blocked(&self, area: &str) -> Option<String> {
        self.stop.clone().or_else(|| match area {
            "account" => self.account_stop.clone(),
            INTERFACE => self.interface_stop.clone(),
            _ => None,
        })
    }

    /// One read through the application interface, on the client of `lent`, which holds no
    /// cookie of the account and is lent its session token for the time of the read.
    ///
    /// The client reads the web site instead whenever the interface does not serve a read,
    /// after the interface refused the token (which the client then forgets), and when the
    /// interface asks to wait. None of these may pass for an answer of the interface:
    /// `expect` looks for what only the interface fills, and what became of the session
    /// while the read ran is told by what the client counts, read before and after it.
    ///
    /// An interface that asks to wait (its block page, a 429) stops the run, like a rate
    /// limit of the web site: see [`Self::interface_asks_to_wait`].
    ///
    /// A refusal is one failed check, and the refused token is not sent again: the account
    /// forgets it too. The next read is lent whatever the account holds then. That is a new
    /// token if the sessions logged in again, as they do once for the next tool call of the
    /// user: a token that did not live until this run then costs one check, not the area.
    /// A second refusal ends the area: two refused requests in a row are what the site was
    /// seen to answer by blocking the address.
    ///
    /// A session its owner dropped while the read ran (the account was linked again, or
    /// unlinked) is not a refusal, and the read is not judged: the check is skipped.
    ///
    /// `web_panel`: the panel of the web site this read is left to for some accounts. The
    /// client carries no web session, so the panel answers it nothing: the check then does
    /// not apply to the account, and the panel is read in the `account` area.
    async fn check_interface<T>(
        &mut self,
        lent: &Lent<'_>,
        name: &str,
        function: &str,
        read: impl Future<Output = Result<T, ClientError>>,
        web_panel: Option<WebPanel>,
        expect: impl FnOnce(&T) -> Result<String, String>,
    ) -> Option<T> {
        const A: &str = INTERFACE;
        if let Some(why) = self.blocked(A) {
            self.skip(A, name, function, &why);
            return None;
        }
        let c = &lent.client;
        let held = lent.lend().await;
        // A block known before the read, from an earlier call of any user of the process or
        // from the login just made: the read would go to the web site under the name of the
        // interface.
        if c.app_blocked() {
            self.interface_asks_to_wait(name, function, true, None);
            return None;
        }
        // Events are counted, where a state would be read too late: a tool call of the same
        // user that logs in again before this read is over leaves no trace of the refusal
        // in the state of the session.
        let (refusals, generation, waits) = (c.app_session_refusals(), c.app_session_generation(), c.app_waits());
        // The token is looked for again once the counts are read: one refused or dropped
        // from here on is counted, and one that went between the two is not lent. Without a
        // token the read would go to the web site under the name of the interface.
        if !held || !c.has_app_session() {
            let why = match &self.refused_at {
                Some(at) => format!(
                    "the application interface refused the session token at `{at}`, and the account obtained no other: nothing more is asked of it"
                ),
                None => "the account no longer holds a session token of the application interface: nothing more is asked of it".to_string(),
            };
            self.skip(A, name, function, &why);
            self.interface_stop = Some(why);
            return None;
        }
        let started = Instant::now();
        let answer = read.await;
        // The site refused the token while this read ran, whatever became of the session
        // since. A session its owner dropped (the account was linked again meanwhile) is not
        // a refusal.
        let refused = c.app_session_refusals() != refusals;
        let dropped = c.app_session_generation() != generation;
        lent.give_back();
        let blocked = c.app_blocked();
        if blocked || c.app_waits() != waits {
            self.interface_asks_to_wait(name, function, blocked, Some(started));
            return None;
        }
        if refused {
            match &self.refused_at {
                None => self.refused_at = Some(name.to_string()),
                Some(first) => {
                    self.interface_stop = Some(format!(
                        "the application interface refused the session token at `{first}`, then the one obtained after it at `{name}`: nothing \
                         more is asked of it in this run"
                    ));
                }
            }
        }
        // An error that names the panel was made by the web site, as is its demand for a login.
        let left_to_the_web = !refused
            && match &answer {
                Err(ClientError::LoginRequired | ClientError::InvalidCredentials) => true,
                Err(e) => web_panel.as_ref().is_some_and(|panel| e.to_string().contains(panel.route)),
                Ok(_) => false,
            };
        match answer {
            // A captcha or a rate limit of the web site stops the whole run as well, on the
            // page that completes the read or on the one read in place of the interface.
            Err(e @ (ClientError::CaptchaRequired | ClientError::RateLimited { .. })) => self.judge(A, name, function, Err(e), expect, started),
            // Whatever came back was read from the web site, or is its error: it says
            // nothing of the interface.
            _ if refused => {
                let detail = "the application interface refused the session token: it is forgotten and not sent again";
                self.push(A, name, function, Outcome::Failed, detail.to_string(), Some(started));
                None
            }
            // The calls that followed the drop were not sent: the web site answered in their
            // place, or nothing did. Neither is the read this check is about.
            _ if dropped => {
                let detail = "the account was linked again or unlinked while this read ran, and its session token went with it: what came back \
                              says nothing of the application interface";
                self.push(A, name, function, Outcome::Skipped, detail.to_string(), Some(started));
                None
            }
            // The token is still held: the read was left to the web site, which wants the
            // account this client does not carry.
            _ if left_to_the_web => {
                let (outcome, detail) = match web_panel {
                    Some(panel) => (
                        Outcome::NotApplicable,
                        format!(
                            "the application interface left this read to the panel of the web site, as it does {} and for any answer whose \
                             meaning was never recorded; the panel is read in the `account` area",
                            panel.when
                        ),
                    ),
                    None => (
                        Outcome::Changed,
                        "the application interface did not answer this read: it was left to the web site, which asks for a login".to_string(),
                    ),
                };
                self.push(A, name, function, outcome, detail, Some(started));
                None
            }
            answer => self.judge(A, name, function, answer, expect, started),
        }
    }

    /// The application interface asks to wait: the site blocks the address of the process
    /// (`blocked`), or a call was rate limited. The client hands such a read over to the web
    /// site, whose answer says nothing of the interface and is not judged. The check is
    /// failed and the run stops, as it does on a rate limit of the web site: every further
    /// check would send its calls, and their retries, to an interface that asked for none.
    fn interface_asks_to_wait(&mut self, name: &str, function: &str, blocked: bool, started: Option<Instant>) {
        let cause = if blocked { "the site blocks the address of this server" } else { "it rate limited a call" };
        let detail = format!(
            "the application interface asks to wait ({cause}): what the web site answers in its place is not judged, and nothing more is sent in \
             this run"
        );
        self.push(INTERFACE, name, function, Outcome::Failed, detail, started);
        self.stop = Some(format!("run stopped after `{name}`: the application interface asks to wait"));
    }

    /// Records the outcome of one read from its answer. Returns the value when the check passed.
    fn judge<T>(
        &mut self,
        area: &str,
        name: &str,
        function: &str,
        answer: Result<T, ClientError>,
        expect: impl FnOnce(&T) -> Result<String, String>,
        started: Instant,
    ) -> Option<T> {
        match answer {
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

    /// A check that does not apply to the account says nothing against the run.
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

/// A listing read from the search index of the application interface. Its rows carry the
/// number of the perfume, which the rows of a web page never do: a row without it was read
/// from the web site in place of the interface, or the rows of the interface changed.
fn index_rows(l: &Listing<PerfumeSummary>, what: &str) -> Result<String, String> {
    if l.items.iter().any(|p| p.name.trim().is_empty() || p.url.is_empty()) {
        return Err("a row has no name or no link".into());
    }
    if l.items.iter().any(|p| p.p_id.is_none()) {
        return Err(format!("a row has no perfume number: {NOT_THE_INTERFACE}"));
    }
    // The tools page through a listing by this count.
    if !l.total_items.is_some_and(|total| total > 0) {
        return Err("the count of matches is missing".into());
    }
    some(l.items.len(), what)
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

/// The record of a perfume read from the application interface. The record of the web page
/// never numbers the brand: one without that number was read from the web site in place of
/// the interface, or the record of the interface changed.
fn interface_record(p: &PerfumeDetails) -> Result<String, String> {
    filled(&p.name, "perfume name")?;
    filled(&p.brand.name, "brand")?;
    if p.brand.numeric_id.is_none() {
        return Err(format!("the brand has no number: {NOT_THE_INTERFACE}"));
    }
    if p.p_id.is_none() {
        return Err("the numeric id is missing".into());
    }
    let notes = p.notes.top.len() + p.notes.heart.len() + p.notes.base.len() + p.notes.flat.len();
    // Unlike the web record, this one says nothing when a part is missing.
    if notes == 0 || p.ratings.is_empty() || p.perfumers.is_empty() || p.classification.is_empty() {
        return Err(format!(
            "{notes} notes, {} ratings, {} perfumers, {} classification groups",
            p.ratings.len(),
            p.perfumers.len(),
            p.classification.len()
        ));
    }
    // The interface knows the page of a perfumer by number only, and learns it here.
    if p.perfumers.iter().any(|perfumer| perfumer.numeric_id.is_none()) {
        return Err("a perfumer has no number".into());
    }
    // The rank label is read on the web page of the perfume: the reader warns when it could not.
    unwarned(&p.warnings)?;
    Ok(format!("{notes} notes, {} ratings, {} classification groups", p.ratings.len(), p.classification.len()))
}

/// A read of the interface that a page of the web site completes, as the canary judges it.
/// The tools get the answer with a warning, whatever kept the page from being read. Here a
/// site that asks to stop (a captcha, a rate limit) or cannot be reached is the error of the
/// read, as it is for a plain web read: it is no changed structure, and the first two stop
/// the run. A page that is gone or can no longer be parsed did change: the answer keeps its
/// warning, which [`unwarned`] reports.
async fn completed<T>(read: impl Future<Output = Result<Completed<T>, ClientError>>) -> Result<T, ClientError> {
    let Completed { value, web_error } = read.await?;
    match web_error {
        Some(e @ (ClientError::CaptchaRequired | ClientError::RateLimited { .. } | ClientError::Upstream(_))) => Err(e),
        _ => Ok(value),
    }
}

/// A read of the interface that a page of the web site completes says so when the page
/// could not be read, or when a part of its own is missing.
fn unwarned(warnings: &[String]) -> Result<(), String> {
    if warnings.is_empty() { Ok(()) } else { Err(format!("the reader warns: {}", warnings.join("; "))) }
}

/// The similar perfumes read from the application interface, which sends the votes with
/// every row. The web list only has the votes of the first `resolved` rows, those whose
/// link it asked for: a list without votes after them was read from the web site in place
/// of the interface, or the list of the interface changed.
fn interface_similar(s: &SimilarPerfumes, resolved: usize) -> Result<String, String> {
    if !s.warnings.is_empty() {
        return Err(format!("the reader warns: {}", s.warnings.join("; ")));
    }
    if s.similar.first().is_some_and(|p| p.url.is_none()) {
        return Err("the first similar perfume has no link".into());
    }
    if !s.similar.iter().skip(resolved).any(|p| p.votes.is_some()) {
        return Err(format!("no row after the first has its votes: {NOT_THE_INTERFACE}"));
    }
    some(s.similar.len(), "similar perfumes")
}

/// Reads through the application interface (`/app/v1`), as the tools make them for a user
/// whose linked account lends its session token: the client of `lent` is lent that token
/// for each read and holds no cookie of the account. Only the reads the interface serves
/// are checked here; the others stay on the web pages, which the `public` area reads.
/// `web_p_id`: the number of the perfume as the web page gives it, for the member's own
/// reads when the record of the interface did not pass.
async fn interface_checks(r: &mut Runner, lent: &Lent<'_>, web_p_id: Option<u64>) {
    let c = &lent.client;
    // The cheapest call first: a token the interface no longer accepts is known at once.
    // This client carries no cookie of the account, so the web site cannot answer here.
    r.check_interface(lent, "own nickname", "own_nick", c.own_nick(), None, |n| filled(n, "nickname").map(|()| "found".to_string())).await;
    let query = SearchQuery { text: Some(SEARCH_TEXT.into()), ..SearchQuery::default() };
    r.check_interface(lent, "search", "search_perfumes", c.search_perfumes(&query, 1), None, |l| index_rows(l, "results")).await;
    let perfume = r.check_interface(lent, "perfume", "get_perfume", completed(c.get_perfume_completed(PERFUME)), None, interface_record).await;
    r.check_interface(lent, "brand", "get_brand", completed(c.get_brand_completed(BRAND, 1)), None, |b| {
        filled(&b.brand.name, "brand name")?;
        // The country is read on the web page of the brand: the reader warns when it could not.
        unwarned(&b.warnings)?;
        index_rows(&b.perfumes, "perfumes")
    })
    .await;
    match perfume.as_ref().and_then(|p| p.perfumers.first()) {
        Some(perfumer) => {
            r.check_interface(lent, "perfumer", "get_perfumer", completed(c.get_perfumer_completed(&perfumer.id, 1)), None, |b| {
                filled(&b.perfumer.name, "perfumer name")?;
                unwarned(&b.warnings)?;
                index_rows(&b.perfumes, "perfumes")
            })
            .await;
        }
        None => r.skip(INTERFACE, "perfumer", "get_perfumer", "needs the perfumer of the `perfume` check"),
    }
    r.check_interface(lent, "release year", "get_rankings", c.get_rankings(&RankingKind::Year(YEAR), 1), None, |l| index_rows(l, "perfumes")).await;
    // The dupes directory is a page of the web site, read in the `public` area.
    let similar = SimilarOptions { resolve_urls: 1, include_layering: false, include_dupes: false };
    r.check_interface(lent, "similar perfumes", "get_similar_perfumes_with", c.get_similar_perfumes_with(PERFUME, similar), None, |s| {
        interface_similar(s, similar.resolve_urls)
    })
    .await;

    // The member's own ratings and classification votes of a perfume: the rate and classify
    // tools read them before and after each write, through the interface for a linked user.
    // The answers have no field of their own to tell the side by. They need none: the web
    // panels answer nothing to this client, which carries no web session.
    let Some(p_id) = perfume.as_ref().and_then(|p| p.p_id).or(web_p_id) else {
        for (name, function) in [("own rating", "my_ratings_of"), ("own classification", "my_classification_of")] {
            r.skip(INTERFACE, name, function, "needs the numeric id of the `perfume` check");
        }
        return;
    };
    let panel = WebPanel { route: routes::RATE_PANEL, when: "for a rating of zero, which only the panel tells from a scale not rated" };
    r.check_interface(lent, "own rating", "my_ratings_of", c.my_ratings_of(p_id), Some(panel), |m| {
        Ok(if m.is_empty() { "not rated".to_string() } else { "rated".to_string() })
    })
    .await;
    let panel = WebPanel { route: routes::CLASSIFY_PANEL, when: "when the account has a vote on this perfume" };
    r.check_interface(lent, "own classification", "my_classification_of", c.my_classification_of(p_id), Some(panel), |l| {
        if l.iter().any(|choice| choice.group.trim().is_empty() || choice.label.trim().is_empty()) {
            return Err("a choice has no group or no label".into());
        }
        some(l.len(), "choices")
    })
    .await;
}

/// What the area says of an account that holds no session token of the application
/// interface, from the way the account is linked.
fn without_session(link: Result<Option<LinkInfo>, SessionError>) -> (Outcome, String) {
    match link {
        Ok(None) => (Outcome::Skipped, "no Parfumo account is linked for this user: nothing is asked of /app/v1".into()),
        // The sessions only log in to the interface for such an account, and have just
        // tried, unless a login that failed a moment ago keeps them from it: every user
        // linked this way is then read on the web site alone, which is worth an alert.
        Ok(Some(link)) if link.mode == "credentials" => (
            Outcome::Failed,
            "the Parfumo account is linked with its e-mail and password and holds no session token of the application interface: its login \
             to the interface gave none (the server log says why), or is not tried again yet after it failed"
                .into(),
        ),
        // The lasting state of a sound setup: no password is held to log in with.
        Ok(Some(_)) => (
            Outcome::NotApplicable,
            "the Parfumo account is linked by the cookie of a browser, which gives no session token of the application interface (an \
             account linked with its e-mail and password gets one): nothing is asked of /app/v1"
                .into(),
        ),
        // `SessionError` never carries a secret.
        Err(e) => (Outcome::Failed, e.to_string()),
    }
}

/// The `interface` area of one run, for the account of `user_id`. Without a token it is one
/// line, which says why: see [`without_session`].
async fn interface_area(r: &mut Runner, sessions: &Sessions, user_id: i64, web_p_id: Option<u64>) {
    const A: &str = INTERFACE;
    let (name, function) = APP_SESSION;
    if let Some(why) = r.stop.clone() {
        r.skip(A, name, function, &why);
        return;
    }
    let client = match sessions.uncached_app_client(user_id) {
        Ok(client) => client,
        Err(e) => return r.push(A, "client", "ParfumoClient::new", Outcome::Failed, e.to_string(), None),
    };
    let lent = Lent { sessions, user_id, client };
    if !lent.lend().await {
        let (outcome, detail) = without_session(sessions.link_info(user_id));
        r.push(A, name, function, outcome, detail, None);
        return;
    }
    interface_checks(r, &lent, web_p_id).await;
}

/// A second client on the web session of the account, without its session token of the
/// application interface, in the request slot of the same user: the client the `account`
/// area reads with.
///
/// The tools read the member's nickname, ratings and classification votes on the web site
/// whenever no token is held (an account linked by cookie, a token the interface refused)
/// and whenever the interface gives an answer it is not trusted with (a vote, a rating of
/// zero). The account client holds a token and would ask the interface: the web panels and
/// their parsers would go unchecked, and the pages of the account would be judged on a
/// nickname the interface gave. The interface has its own area.
///
/// The account client itself keeps its token: taking it away, even for a moment, would
/// take it from the tool calls of the same user that run meanwhile.
fn without_token(sessions: &Sessions, user_id: i64, account: &ParfumoClient) -> Result<ParfumoClient, String> {
    let web = sessions.uncached_public_client(user_id).map_err(|e| e.to_string())?;
    web.import_cookies(&account.export_cookies());
    // The token is kept with the cookies: it is dropped from the copy before anything is
    // sent. A copy that came without one is left alone: nothing is forgotten on its behalf.
    if web.has_app_session() {
        web.forget_app_session();
    }
    if web.has_app_session() {
        return Err("the copy of the account session still holds a token of the application interface".into());
    }
    Ok(web)
}

/// Reads of the linked (test) account on the web site. Lists may be empty: the parsers
/// still check the structure of the page around them. `c` holds the web session of the
/// account and no token of the application interface (see [`without_token`]): the
/// nickname, the rating panel and the classification panel are read on the web site, and
/// the member's own pages are asked under the nickname of the web site.
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
/// public pages, through the application interface and then on the web site (`None`:
/// public pages only). The report is stored and logged.
pub async fn run(state: &ServerState, user: Option<&db::User>) -> Report {
    let started_at = db::now();
    let sessions = &state.core.sessions;
    let mut r = Runner::default();
    // Limiter slot of the canary user, or of nobody (0): the global budget applies anyway.
    let slot = user.map(|u| u.id).unwrap_or(0);
    let mut p_id = None;
    // This client is never lent a token: it reads the web pages, whatever the account holds.
    match sessions.uncached_public_client(slot) {
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
        interface_area(&mut r, sessions, user.id, p_id).await;
        if let Some(why) = r.stop.clone() {
            r.skip("account", "account session", "account_client", &why);
        } else {
            // No tool call of this user may follow: a session of the application interface
            // that the site ends during the run is forgotten in the database when the run
            // ends, not later.
            let _stored = sessions.storing_app_session(user.id);
            match sessions.account_client(user.id).await {
                Ok(client) => match without_token(sessions, user.id, &client) {
                    Ok(web) => account_checks(&mut r, &web, p_id).await,
                    Err(why) => r.push("account", "web session", "uncached_public_client", Outcome::Failed, why, None),
                },
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

    /// What the canary makes of the page that completes a read of the application
    /// interface: the tools get a warning for all of these.
    #[tokio::test]
    async fn a_completing_page_that_asks_to_stop_or_cannot_be_reached_is_the_error_of_the_read() {
        let read = |web_error| completed(async move { Ok::<_, ClientError>(Completed { value: 1, web_error }) });
        assert_eq!(read(None).await, Ok(1));
        for e in [
            ClientError::CaptchaRequired,
            ClientError::RateLimited { retry_after_secs: None },
            ClientError::Upstream("HTTP 503 on /Perfumes/Dior".into()),
        ] {
            assert_eq!(read(Some(e.clone())).await, Err(e), "judged as a web read is: failed, and the first two stop the run");
        }
        // The page changed: the answer is judged, and its warning says what is missing.
        for e in [ClientError::NotFound("/Perfumes/Dior".into()), ClientError::ParserOutdated("brand head".into()), ClientError::LoginRequired] {
            assert_eq!(read(Some(e)).await, Ok(1));
        }
        let failed = completed(async { Err::<Completed<u32>, _>(ClientError::NotFound("/perfume_details/1".into())) });
        assert_eq!(failed.await, Err(ClientError::NotFound("/perfume_details/1".into())), "the error of the interface is its own");
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
        assert!(text.contains("CHANGED  public     perfume") && text.contains("[get_perfume]"), "{text}");
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

#[cfg(test)]
mod site_tests;
