//! Account tools of jalon 3: ratings, classification, reviews and statements (section 7.4).
//!
//! Ratings, classification votes, reviews and statements are seen by other Parfumo members
//! (type P). The five tools that send them stay a dry run unless the operator sets
//! `PARFUMO_MCP_PUBLIC_WRITES`; they still read the current state, so a dry run shows what
//! would change. Deleting one's own review or statement (type D) is not held back.
//!
//! What was measured and what was not (DISCOVERY §3.4, §3.5, §11): rating and classification
//! routes were exercised on the test account; the review and statement routes were only read
//! in the site's forms and scripts and have never been sent.

use std::collections::BTreeMap;

use parfumo_client::{ClassChoice, MemberRatings, OwnDraft, OwnText, RatedPerfume, TextKind, WriteCall, calls, site::routes};
use rmcp::{
    ErrorData,
    handler::server::{common::schema_for_output, wrapper::Parameters},
    model::CallToolResult,
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    account_tools::{chars_at_most, planned_text},
    caller::Caller,
    errors::{ErrorCode, ToolError},
    paging::{self, MAX_STRUCTURED_CHARS},
    public_gate::{GATE_NOTE, NEVER_SENT_NOTE, OWN_WORDS_NOTE},
    tools::{Clients, ParfumoTools},
};

const DELETE_TOKEN_PLACEHOLDER: &str = "<token of the form's delete button>";

// ---- parameters ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// How good it smells.
    #[default]
    Scent,
    /// How long it lasts.
    Longevity,
    /// How far it projects.
    Sillage,
    /// The bottle.
    Bottle,
    /// Value for money.
    ValueForMoney,
}

impl Scale {
    const ALL: [Scale; 5] = [Scale::Scent, Scale::Longevity, Scale::Sillage, Scale::Bottle, Scale::ValueForMoney];

    /// Name of the scale on Parfumo.
    fn site(self) -> &'static str {
        match self {
            Scale::Scent => "scent",
            Scale::Longevity => "durability",
            Scale::Sillage => "sillage",
            Scale::Bottle => "bottle",
            Scale::ValueForMoney => "pricing",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scale::Scent => "scent",
            Scale::Longevity => "longevity",
            Scale::Sillage => "sillage",
            Scale::Bottle => "bottle",
            Scale::ValueForMoney => "value_for_money",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RatePerfumeParams {
    /// Perfume: the `id` returned by other tools (e.g. "Dior/Sauvage_Eau_de_Toilette"), its Parfumo URL, or its numeric `p_id`.
    pub perfume: String,
    /// Scent rating from 0 to 10, in steps of 0.5.
    #[serde(default)]
    pub scent: Option<f64>,
    /// Longevity rating from 0 to 10, in steps of 0.5.
    #[serde(default)]
    pub longevity: Option<f64>,
    /// Sillage rating from 0 to 10, in steps of 0.5.
    #[serde(default)]
    pub sillage: Option<f64>,
    /// Bottle rating from 0 to 10, in steps of 0.5.
    #[serde(default)]
    pub bottle: Option<f64>,
    /// Value-for-money rating from 0 to 10, in steps of 0.5.
    #[serde(default)]
    pub value_for_money: Option<f64>,
    /// Scales whose rating is withdrawn.
    #[serde(default)]
    pub remove: Vec<Scale>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyPerfumeParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// Labels to vote for, e.g. ["Fresh", "Spring", "Daily"]. Call the tool without `add` and `remove` to get the labels Parfumo offers.
    #[serde(default)]
    pub add: Vec<String>,
    /// Labels whose vote is withdrawn.
    #[serde(default)]
    pub remove: Vec<String>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RatingSort {
    /// Most recently rated first.
    #[default]
    Newest,
    /// Oldest rating first.
    Oldest,
    /// Highest rating first.
    Highest,
    /// Lowest rating first.
    Lowest,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMyRatingsParams {
    /// Which rating scale to list (default `scent`). Parfumo lists one scale at a time.
    #[serde(default)]
    pub scale: Option<Scale>,
    /// Sort order (default `newest`).
    #[serde(default)]
    pub sort: Option<RatingSort>,
    /// Maximum number of items to return (default 25, maximum 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque pagination cursor: the `next_cursor` of the previous response.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteReviewParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// Title of the review (200 characters at most).
    pub title: String,
    /// Text of the review, in the user's own words (7500 characters at most).
    pub text: String,
    /// When true, nothing is sent: the answer shows the request that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteStatementParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// Text of the statement, in the user's own words (140 characters at most).
    pub text: String,
    /// When true, nothing is sent: the answer shows the request that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTextParams {
    /// Perfume the review or statement is about: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// What to delete: `review` or `statement`.
    pub kind: TextKind,
    /// When true, nothing is sent: the answer shows the request that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMyReviewsParams {
    /// `review` (default) for long reviews, `statement` for statements.
    #[serde(default)]
    pub kind: Option<TextKind>,
    /// Maximum number of items to return (default 25, maximum 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque pagination cursor: the `next_cursor` of the previous response.
    #[serde(default)]
    pub cursor: Option<String>,
}

// ---- outputs -------------------------------------------------------------------------------

/// The member's votes in one classification group.
#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct ClassVotes {
    /// `Type`, `Style`, `Season` or `Occasion`.
    pub group: String,
    /// Labels the member voted for.
    pub voted: Vec<String>,
    /// Every label Parfumo offers in this group.
    pub options: Vec<String>,
}

/// Result of a rating, classification, review or statement write.
#[derive(Debug, Serialize, JsonSchema, Default)]
pub struct FeedbackWriteOutput {
    /// True when nothing was sent to Parfumo.
    pub dry_run: bool,
    /// True when at least one write request was sent.
    pub executed: bool,
    /// Outcome: `rated`, `classified`, `published`, `updated`, `deleted`, `unchanged`, `absent`, `read`, `unverified`, `still_present` or `dry_run`.
    pub status: String,
    /// Whether this server is allowed to send publicly visible writes (operator setting).
    pub public_writes_enabled: bool,
    /// The write requests sent, or that would be sent (CSRF and form tokens omitted).
    pub planned_requests: Vec<WriteCall>,
    /// The perfume, as passed in.
    pub perfume: String,
    /// Numeric Parfumo perfume id.
    pub p_id: u64,
    /// The member's ratings of the perfume (0-10) as Parfumo holds them after the call; before it in a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_ratings: Option<MemberRatings>,
    /// The member's classification votes as Parfumo holds them after the call; before it in a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_classification: Option<Vec<ClassVotes>>,
    /// The member's review or statement as Parfumo holds it after the call; before it in a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_text: Option<OwnDraft>,
    /// Side effects and things to know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MyRatingsOutput {
    /// Scale listed: `scent`, `longevity`, `sillage`, `bottle` or `value_for_money`.
    pub scale: Scale,
    /// Perfumes the member rated on this scale.
    pub items: Vec<RatedPerfume>,
    /// Number of perfumes rated on this scale, when the site states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_items: Option<u32>,
    /// Pass as `cursor` to get the following items. Absent when there is nothing more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MyTextsOutput {
    /// `review` or `statement`.
    pub kind: TextKind,
    /// The member's own reviews or statements, newest first.
    pub items: Vec<OwnText>,
    /// Number of reviews or statements, when the site states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_items: Option<u32>,
    /// Pass as `cursor` to get the following items. Absent when there is nothing more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- helpers -------------------------------------------------------------------------------

/// A rating from 0 to 10 in steps of 0.5 → the site's 0-100 value.
fn rating_tenths(scale: Scale, value: f64) -> Result<u32, ToolError> {
    let doubled = value * 2.0;
    if !value.is_finite() || !(0.0..=10.0).contains(&value) || (doubled - doubled.round()).abs() > 1e-9 {
        return Err(ToolError::invalid(format!("`{}` must be a rating from 0 to 10 in steps of 0.5.", scale.label())));
    }
    Ok((value * 10.0).round() as u32)
}

fn ratings_line(r: &MemberRatings) -> String {
    let parts: Vec<String> = Scale::ALL.iter().filter_map(|s| r.get(s.site()).map(|v| format!("{} {v}", s.label()))).collect();
    if parts.is_empty() { "no rating".into() } else { parts.join(", ") }
}

fn group_votes(choices: &[ClassChoice]) -> Vec<ClassVotes> {
    let mut out: Vec<ClassVotes> = Vec::new();
    for c in choices {
        if out.last().is_none_or(|g| g.group != c.group) {
            out.push(ClassVotes { group: c.group.clone(), voted: Vec::new(), options: Vec::new() });
        }
        let g = out.last_mut().expect("just pushed");
        g.options.push(c.label.clone());
        if c.voted {
            g.voted.push(c.label.clone());
        }
    }
    out
}

fn votes_line(groups: &[ClassVotes]) -> String {
    let parts: Vec<String> = groups.iter().filter(|g| !g.voted.is_empty()).map(|g| format!("{}: {}", g.group, g.voted.join(", "))).collect();
    if parts.is_empty() { "no classification vote".into() } else { parts.join("; ") }
}

fn find_choice<'a>(choices: &'a [ClassChoice], label: &str) -> Result<&'a ClassChoice, ToolError> {
    let wanted = label.trim().to_lowercase();
    // `Occasion: Daily` is accepted as well as `Daily`.
    let (group, name) = match wanted.split_once([':', '/']) {
        Some((g, n)) => (Some(g.trim().to_string()), n.trim().to_string()),
        None => (None, wanted.clone()),
    };
    let hits: Vec<&ClassChoice> =
        choices.iter().filter(|c| c.label.to_lowercase() == name && group.as_ref().is_none_or(|g| c.group.to_lowercase() == *g)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(ToolError::invalid(format!(
            "Parfumo has no classification label '{}'. Labels: {}.",
            label.trim(),
            group_votes(choices).iter().map(|g| format!("{} ({})", g.group, g.options.join(", "))).collect::<Vec<_>>().join("; ")
        ))),
        _ => Err(ToolError::invalid(format!("'{}' exists in several groups: write it as 'Group: Label'.", label.trim()))),
    }
}

/// Texts are compared without regard to the whitespace the site may normalise.
fn same_text(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

fn kind_word(kind: TextKind) -> &'static str {
    match kind {
        TextKind::Review => "review",
        TextKind::Statement => "statement",
    }
}

fn feedback_text(out: &FeedbackWriteOutput) -> String {
    let mut text = if out.dry_run {
        format!("Dry run: nothing was sent to Parfumo. Requests that would be made:\n{}", planned_text(&out.planned_requests))
    } else {
        format!("Status: {}.", out.status)
    };
    let state = if out.dry_run { "Current" } else { "Now" };
    if let Some(r) = &out.my_ratings {
        text.push_str(&format!("\n{state} ratings: {}", ratings_line(r)));
    }
    if let Some(groups) = &out.my_classification {
        text.push_str(&format!("\n{state} votes: {}", votes_line(groups)));
    }
    if let Some(t) = &out.my_text {
        let (excerpt, cut) = paging::truncate_chars(&t.text, 200);
        text.push_str(&match (t.text.is_empty(), &t.title) {
            (true, _) => format!("\n{state} text on Parfumo: none"),
            (false, Some(title)) => format!("\n{state} text on Parfumo: \"{title}\" — {excerpt}{}", if cut { "…" } else { "" }),
            (false, None) => format!("\n{state} text on Parfumo: {excerpt}{}", if cut { "…" } else { "" }),
        });
    }
    for n in &out.notes {
        text.push_str(&format!("\nNote: {n}"));
    }
    text
}

fn own_text_line(t: &OwnText) -> String {
    let (excerpt, cut) = paging::truncate_chars(&t.text, 160);
    format!(
        "{}{} — {}{}{} · id: {}",
        t.perfume_name,
        t.brand.as_deref().map(|b| format!(" ({b})")).unwrap_or_default(),
        t.title.as_deref().map(|title| format!("\"{title}\" ")).unwrap_or_default(),
        excerpt.replace('\n', " "),
        if cut { "…" } else { "" },
        t.perfume_id
    )
}

// ---- tools ---------------------------------------------------------------------------------

impl ParfumoTools {
    fn base_output(&self, perfume: &str, p_id: u64, dry_run: bool) -> FeedbackWriteOutput {
        let enabled = self.state.cfg.public_writes;
        let mut out = FeedbackWriteOutput { dry_run, public_writes_enabled: enabled, perfume: perfume.to_string(), p_id, ..Default::default() };
        if dry_run {
            out.status = "dry_run".into();
        }
        out
    }

    /// Publishes or replaces the member's review or statement on a perfume.
    async fn save_text(
        &self,
        c: &Clients,
        kind: TextKind,
        perfume: &str,
        title: Option<&str>,
        text: &str,
        asked_dry_run: bool,
    ) -> Result<(String, FeedbackWriteOutput), ToolError> {
        let (dry_run, public_writes) = self.public_gate(asked_dry_run);
        let p_id = self.resolve_p_id(c, perfume).await?;
        let current = c.account.my_text_on(kind, p_id).await.map_err(|e| self.ce(e))?;
        let exists = !current.text.is_empty();
        if exists && same_text(&current.text, text) && title.is_none_or(|t| current.title.as_deref().is_some_and(|ct| same_text(ct, t))) {
            let out = FeedbackWriteOutput { status: "unchanged".into(), my_text: Some(current), ..self.base_output(perfume, p_id, false) };
            return Ok((format!("Parfumo already holds this {} for the perfume: nothing was sent.", kind_word(kind)), out));
        }
        let planned = vec![match kind {
            TextKind::Review => calls::review_save(p_id, title.unwrap_or_default(), text),
            TextKind::Statement => calls::statement_save(p_id, text),
        }];
        let mut notes = vec![
            format!("A {} is public: every Parfumo member can read it under the perfume and on the member's profile.", kind_word(kind)),
            OWN_WORDS_NOTE.to_string(),
        ];
        if exists {
            notes.push(format!("The member already wrote a {} on this perfume: it is replaced.", kind_word(kind)));
        }
        if dry_run {
            if !public_writes {
                notes.insert(0, GATE_NOTE.to_string());
            }
            let out = FeedbackWriteOutput { planned_requests: planned, my_text: Some(current), notes, ..self.base_output(perfume, p_id, true) };
            return Ok((feedback_text(&out), out));
        }
        let tool = match kind {
            TextKind::Review => "parfumo_write_review",
            TextKind::Statement => "parfumo_write_statement",
        };
        self.send_public(c, tool, &planned).await?;
        let after = c.account.my_text_on(kind, p_id).await.map_err(|e| self.ce(e))?;
        notes.push(NEVER_SENT_NOTE.to_string());
        let status = if !same_text(&after.text, text) {
            notes.push("Parfumo accepted the request, but reading the text back does not return what was sent.".into());
            "unverified"
        } else if exists {
            "updated"
        } else {
            "published"
        };
        let out = FeedbackWriteOutput {
            executed: true,
            status: status.into(),
            planned_requests: planned,
            my_text: Some(after),
            notes,
            ..self.base_output(perfume, p_id, false)
        };
        Ok((feedback_text(&out), out))
    }
}

#[tool_router(router = feedback_router, vis = "pub(crate)")]
impl ParfumoTools {
    #[tool(
        name = "parfumo_rate_perfume",
        description = "Rate a perfume on the user's own Parfumo account on any of the five scales (scent, longevity, sillage, bottle, value for money), from 0 to 10 in steps of 0.5, or withdraw ratings with `remove`. PUBLIC: ratings count in the community average and are shown to other members. A new value replaces the previous rating. Returns the user's ratings of the perfume as Parfumo holds them after the call. On this server publicly visible writes stay in dry-run mode unless the operator enabled them; the answer then shows the current ratings and the requests that would be made. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Rate a perfume", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<FeedbackWriteOutput>()
    )]
    async fn rate_perfume(&self, Parameters(p): Parameters<RatePerfumeParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let (dry_run, public_writes) = self.public_gate(p.dry_run);
        let target = format!("{} dry_run={dry_run}", p.perfume);
        self.run_account(&caller, "parfumo_rate_perfume", &target, |c| async move {
            let given = [
                (Scale::Scent, p.scent),
                (Scale::Longevity, p.longevity),
                (Scale::Sillage, p.sillage),
                (Scale::Bottle, p.bottle),
                (Scale::ValueForMoney, p.value_for_money),
            ];
            let mut wanted: BTreeMap<Scale, u32> = BTreeMap::new();
            for (scale, value) in given {
                if let Some(v) = value {
                    wanted.insert(scale, rating_tenths(scale, v)?);
                }
            }
            if wanted.is_empty() && p.remove.is_empty() {
                return Err(ToolError::invalid(
                    "Give at least one rating (scent, longevity, sillage, bottle, value_for_money) or a scale to `remove`.",
                ));
            }
            if let Some(both) = p.remove.iter().find(|s| wanted.contains_key(s)) {
                return Err(ToolError::invalid(format!("`{}` is both rated and removed in the same call.", both.label())));
            }
            let p_id = self.resolve_p_id(&c, &p.perfume).await?;
            let current = c.account.my_ratings_of(p_id).await.map_err(|e| self.ce(e))?;
            let mut planned = Vec::new();
            for (scale, tenths) in &wanted {
                if current.get(scale.site()) != Some(*tenths as f32 / 10.0) {
                    planned.push(calls::vote_add(p_id, scale.site(), *tenths));
                }
            }
            for scale in Scale::ALL.iter().filter(|s| p.remove.contains(s)) {
                if current.get(scale.site()).is_some() {
                    planned.push(calls::vote_remove(p_id, scale.site()));
                }
            }
            if planned.is_empty() {
                let out = FeedbackWriteOutput { status: "unchanged".into(), my_ratings: Some(current), ..self.base_output(&p.perfume, p_id, false) };
                return Ok((
                    format!(
                        "Parfumo already holds these ratings: nothing was sent.\nRatings: {}",
                        ratings_line(out.my_ratings.as_ref().expect("set above"))
                    ),
                    out,
                ));
            }
            let mut notes =
                vec!["A rating is public: it counts in the perfume's community average and other members can see who rated it.".to_string()];
            if dry_run {
                if !public_writes {
                    notes.insert(0, GATE_NOTE.to_string());
                }
                let out =
                    FeedbackWriteOutput { planned_requests: planned, my_ratings: Some(current), notes, ..self.base_output(&p.perfume, p_id, true) };
                return Ok((feedback_text(&out), out));
            }
            self.send_public(&c, "parfumo_rate_perfume", &planned).await?;
            let after = c.account.my_ratings_of(p_id).await.map_err(|e| self.ce(e))?;
            let mut wrong: Vec<&str> =
                wanted.iter().filter(|(s, t)| after.get(s.site()) != Some(**t as f32 / 10.0)).map(|(s, _)| s.label()).collect();
            wrong.extend(p.remove.iter().filter(|s| after.get(s.site()).is_some()).map(|s| s.label()));
            let status = if wrong.is_empty() {
                "rated"
            } else {
                notes.push(format!("Parfumo accepted the requests, but reading the ratings back differs for: {}.", wrong.join(", ")));
                "unverified"
            };
            let out = FeedbackWriteOutput {
                executed: true,
                status: status.into(),
                planned_requests: planned,
                my_ratings: Some(after),
                notes,
                ..self.base_output(&p.perfume, p_id, false)
            };
            Ok((feedback_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_classify_perfume",
        description = "Vote on the community classification of a perfume from the user's own Parfumo account: type (Fresh, Woody...), style, season and occasion. Give labels to `add` and/or to `remove`; called with neither, it only returns the labels Parfumo offers and the user's current votes. PUBLIC: votes feed the classification chart shown on the perfume page. Returns the user's votes as Parfumo holds them after the call. On this server publicly visible writes stay in dry-run mode unless the operator enabled them; the answer then shows the current votes and the requests that would be made. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Classify a perfume", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<FeedbackWriteOutput>()
    )]
    async fn classify_perfume(&self, Parameters(p): Parameters<ClassifyPerfumeParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let (dry_run, public_writes) = self.public_gate(p.dry_run);
        let target = format!("{} +{:?} -{:?} dry_run={dry_run}", p.perfume, p.add, p.remove);
        self.run_account(&caller, "parfumo_classify_perfume", &target, |c| async move {
            let p_id = self.resolve_p_id(&c, &p.perfume).await?;
            let choices = c.account.my_classification_of(p_id).await.map_err(|e| self.ce(e))?;
            let add: Vec<&ClassChoice> = p.add.iter().filter(|l| !l.trim().is_empty()).map(|l| find_choice(&choices, l)).collect::<Result<_, _>>()?;
            let remove: Vec<&ClassChoice> =
                p.remove.iter().filter(|l| !l.trim().is_empty()).map(|l| find_choice(&choices, l)).collect::<Result<_, _>>()?;
            if let Some(both) = add.iter().find(|a| remove.iter().any(|r| r.label_id == a.label_id && r.group_id == a.group_id)) {
                return Err(ToolError::invalid(format!("'{}' is both added and removed in the same call.", both.label)));
            }
            if add.is_empty() && remove.is_empty() {
                let out = FeedbackWriteOutput {
                    status: "read".into(),
                    my_classification: Some(group_votes(&choices)),
                    ..self.base_output(&p.perfume, p_id, false)
                };
                let groups = out.my_classification.as_ref().expect("set above");
                let labels = groups.iter().map(|g| format!("{}: {}", g.group, g.options.join(", "))).collect::<Vec<_>>().join("\n");
                return Ok((format!("Nothing was sent. Current votes: {}.\nLabels offered by Parfumo:\n{labels}", votes_line(groups)), out));
            }
            let mut planned: Vec<WriteCall> = Vec::new();
            let mut seen: Vec<(u64, u64)> = Vec::new();
            for (choice, adding) in add.iter().map(|c| (c, true)).chain(remove.iter().map(|c| (c, false))) {
                if choice.voted != adding && !seen.contains(&(choice.group_id, choice.label_id)) {
                    seen.push((choice.group_id, choice.label_id));
                    planned.push(calls::classification_vote(p_id, choice.group_id, choice.label_id, adding));
                }
            }
            if planned.is_empty() {
                let out = FeedbackWriteOutput {
                    status: "unchanged".into(),
                    my_classification: Some(group_votes(&choices)),
                    ..self.base_output(&p.perfume, p_id, false)
                };
                return Ok((
                    format!(
                        "Parfumo already holds these votes: nothing was sent.\nVotes: {}",
                        votes_line(out.my_classification.as_ref().expect("set above"))
                    ),
                    out,
                ));
            }
            let mut notes = vec!["A classification vote is public: it feeds the chart shown on the perfume page.".to_string()];
            if dry_run {
                if !public_writes {
                    notes.insert(0, GATE_NOTE.to_string());
                }
                let out = FeedbackWriteOutput {
                    planned_requests: planned,
                    my_classification: Some(group_votes(&choices)),
                    notes,
                    ..self.base_output(&p.perfume, p_id, true)
                };
                return Ok((feedback_text(&out), out));
            }
            self.send_public(&c, "parfumo_classify_perfume", &planned).await?;
            let after = c.account.my_classification_of(p_id).await.map_err(|e| self.ce(e))?;
            let voted = |choice: &ClassChoice| after.iter().any(|a| a.group_id == choice.group_id && a.label_id == choice.label_id && a.voted);
            let mut wrong: Vec<&str> = add.iter().filter(|c| !voted(c)).map(|c| c.label.as_str()).collect();
            wrong.extend(remove.iter().filter(|c| voted(c)).map(|c| c.label.as_str()));
            let status = if wrong.is_empty() {
                "classified"
            } else {
                notes.push(format!("Parfumo accepted the requests, but reading the votes back differs for: {}.", wrong.join(", ")));
                "unverified"
            };
            let out = FeedbackWriteOutput {
                executed: true,
                status: status.into(),
                planned_requests: planned,
                my_classification: Some(group_votes(&after)),
                notes,
                ..self.base_output(&p.perfume, p_id, false)
            };
            Ok((feedback_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_list_my_ratings",
        description = "List the perfumes the user rated on their own Parfumo account, one rating scale at a time (scent by default; longevity, sillage, bottle, value_for_money), newest first or sorted by rating. Returns a paginated list with the perfume `id` and the rating out of 10. For all five ratings of the perfumes of a collection list, parfumo_get_collection is quicker. Requires a linked Parfumo account.",
        annotations(title = "List my ratings", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<MyRatingsOutput>()
    )]
    async fn list_my_ratings(&self, Parameters(p): Parameters<ListMyRatingsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let scale = p.scale.unwrap_or_default();
        self.run_account(&caller, "parfumo_list_my_ratings", scale.label(), |c| async move {
            let order = match p.sort.unwrap_or_default() {
                RatingSort::Newest => "d_desc",
                RatingSort::Oldest => "d_asc",
                RatingSort::Highest => "v_desc",
                RatingSort::Lowest => "v_asc",
            };
            let start = paging::page_cursor(p.cursor.as_deref())?;
            let account = &c.account;
            let collected = paging::collect_pages(start, paging::clamp_limit(p.limit), |page| async move {
                account.my_ratings_page(scale.site(), order, page).await.map(|l| (l, ()))
            })
            .await
            .map_err(|e| self.ce(e))?;
            let (items, next_cursor, total_items, ()) = paging::finish_pages(collected, MAX_STRUCTURED_CHARS - 1_000);
            let header = format!(
                "{} perfume(s) rated for {}{}.{}",
                items.len(),
                scale.label(),
                total_items.map(|t| format!(" (of {t})")).unwrap_or_default(),
                if next_cursor.is_some() { " More available: pass next_cursor as cursor." } else { "" }
            );
            let lines = items.iter().map(|r| {
                format!("{} — {}{} · id: {}", r.rating, r.name, r.brand.as_deref().map(|b| format!(" ({b})")).unwrap_or_default(), r.perfume_id)
            });
            let text = paging::digest(header, lines.collect::<Vec<_>>());
            Ok((text, MyRatingsOutput { scale, items, total_items, next_cursor }))
        })
        .await
    }

    #[tool(
        name = "parfumo_write_review",
        description = "Publish a long review (title and text) of a perfume from the user's own Parfumo account, or replace the review the user already wrote on that perfume. PUBLIC and subject to Parfumo's review rules. Parfumo forbids AI-generated review text and restricts or deletes accounts that post it: only pass on text the user wrote themselves, unchanged; never compose or rewrite the review. Returns the review as Parfumo holds it after the call. On this server publicly visible writes stay in dry-run mode unless the operator enabled them; the answer then shows the request that would be made. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Write a review", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<FeedbackWriteOutput>()
    )]
    async fn write_review(&self, Parameters(p): Parameters<WriteReviewParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let (dry_run, _) = self.public_gate(p.dry_run);
        let target = format!("{} title={} chars text={} chars dry_run={dry_run}", p.perfume, p.title.chars().count(), p.text.chars().count());
        self.run_account(&caller, "parfumo_write_review", &target, |c| async move {
            let (title, text) = (p.title.trim(), p.text.trim());
            if title.is_empty() || text.is_empty() {
                return Err(ToolError::invalid("A review needs both a `title` and a `text`."));
            }
            chars_at_most("title", title, routes::REVIEW_TITLE_MAX)?;
            chars_at_most("text", text, routes::REVIEW_TEXT_MAX)?;
            self.save_text(&c, TextKind::Review, &p.perfume, Some(title), text, p.dry_run).await
        })
        .await
    }

    #[tool(
        name = "parfumo_write_statement",
        description = "Publish a statement (short review, 140 characters at most) on a perfume from the user's own Parfumo account, or replace the statement the user already wrote on that perfume. PUBLIC and subject to Parfumo's review rules. Parfumo forbids AI-generated text and restricts or deletes accounts that post it: only pass on text the user wrote themselves, unchanged. Returns the statement as Parfumo holds it after the call. On this server publicly visible writes stay in dry-run mode unless the operator enabled them; the answer then shows the request that would be made. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Write a statement", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<FeedbackWriteOutput>()
    )]
    async fn write_statement(&self, Parameters(p): Parameters<WriteStatementParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let (dry_run, _) = self.public_gate(p.dry_run);
        let target = format!("{} text={} chars dry_run={dry_run}", p.perfume, p.text.chars().count());
        self.run_account(&caller, "parfumo_write_statement", &target, |c| async move {
            let text = p.text.trim();
            if text.is_empty() {
                return Err(ToolError::invalid("`text` is empty. Use parfumo_delete_review_or_statement to remove a statement."));
            }
            chars_at_most("text", text, routes::STATEMENT_MAX)?;
            self.save_text(&c, TextKind::Statement, &p.perfume, None, text, p.dry_run).await
        })
        .await
    }

    #[tool(
        name = "parfumo_delete_review_or_statement",
        description = "Delete the review or the statement the user wrote on a perfume, from the user's own Parfumo account. Cannot be undone: the text, its awards and its comments are lost. Deleting a text that does not exist is not an error. Returns whether Parfumo still holds the text after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Delete my review or statement", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<FeedbackWriteOutput>()
    )]
    async fn delete_review_or_statement(&self, Parameters(p): Parameters<DeleteTextParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let kind = p.kind;
        let target = format!("{} of {} dry_run={}", kind_word(kind), p.perfume, p.dry_run);
        self.run_account(&caller, "parfumo_delete_review_or_statement", &target, |c| async move {
            let (p_id, slug) = c.public.perfume_numeric_id(&p.perfume).await.map_err(|e| self.ce(e))?;
            let current = c.account.my_text_on(kind, p_id).await.map_err(|e| self.ce(e))?;
            if current.text.is_empty() {
                let out = FeedbackWriteOutput { status: "absent".into(), my_text: Some(current), ..self.base_output(&p.perfume, p_id, false) };
                return Ok((format!("The user has no {} on this perfume: nothing was sent.", kind_word(kind)), out));
            }
            // The delete button carries the id and a token. The form is first asked without
            // an id; if it shows no button, the id is looked up on the member's own pages.
            let mut handle = c.account.delete_handle(kind, p_id, 0).await.map_err(|e| self.ce(e))?;
            if handle.is_none()
                && let Some(slug) = &slug
            {
                'pages: for page in 1..=paging::MAX_SITE_PAGES_PER_CALL {
                    let listing = c.account.my_texts_page(kind, page).await.map_err(|e| self.ce(e))?;
                    if let Some(own) = listing.items.iter().find(|t| t.perfume_id.eq_ignore_ascii_case(slug)) {
                        handle = c.account.delete_handle(kind, p_id, own.id).await.map_err(|e| self.ce(e))?;
                        break 'pages;
                    }
                    if listing.items.is_empty() || listing.total_pages.is_some_and(|t| page >= t) {
                        break;
                    }
                }
            }
            let Some(handle) = handle else {
                return Err(ToolError::new(
                    ErrorCode::UpstreamError,
                    format!("Parfumo holds a {} of the user on this perfume, but its form shows no delete button to this server. Nothing was sent; delete it on parfumo.com.", kind_word(kind)),
                ));
            };
            let shown = vec![calls::text_delete(kind, handle.id, handle.p_id, DELETE_TOKEN_PLACEHOLDER)];
            let mut notes = vec![format!("Deleting a {} cannot be undone.", kind_word(kind))];
            if p.dry_run {
                let out = FeedbackWriteOutput { planned_requests: shown, my_text: Some(current), notes, ..self.base_output(&p.perfume, p_id, true) };
                return Ok((feedback_text(&out), out));
            }
            self.send_all(&c, &[calls::text_delete(kind, handle.id, handle.p_id, &handle.h)]).await?;
            let after = c.account.my_text_on(kind, p_id).await.map_err(|e| self.ce(e))?;
            notes.push(NEVER_SENT_NOTE.to_string());
            let status = if after.text.is_empty() {
                "deleted"
            } else {
                notes.push("Parfumo answered, but the text is still there when read back.".into());
                "still_present"
            };
            let out = FeedbackWriteOutput { executed: true, status: status.into(), planned_requests: shown, my_text: Some(after), notes, ..self.base_output(&p.perfume, p_id, false) };
            Ok((feedback_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_list_my_reviews",
        description = "List the reviews (default) or the statements the user wrote on their own Parfumo account, newest first, with the perfume `id`, the full text, the ratings shown with it and the number of awards and comments. Returns a paginated list. Requires a linked Parfumo account.",
        annotations(title = "List my reviews and statements", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<MyTextsOutput>()
    )]
    async fn list_my_reviews(&self, Parameters(p): Parameters<ListMyReviewsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let kind = p.kind.unwrap_or_default();
        self.run_account(&caller, "parfumo_list_my_reviews", kind_word(kind), |c| async move {
            let start = paging::page_cursor(p.cursor.as_deref())?;
            let account = &c.account;
            let collected =
                paging::collect_pages(
                    start,
                    paging::clamp_limit(p.limit),
                    |page| async move { account.my_texts_page(kind, page).await.map(|l| (l, ())) },
                )
                .await
                .map_err(|e| self.ce(e))?;
            let (items, next_cursor, total_items, ()) = paging::finish_pages(collected, MAX_STRUCTURED_CHARS - 1_000);
            let header = format!(
                "{} {}(s){}.{}",
                items.len(),
                kind_word(kind),
                total_items.map(|t| format!(" of {t}")).unwrap_or_default(),
                if next_cursor.is_some() { " More available: pass next_cursor as cursor." } else { "" }
            );
            let text = paging::digest(header, items.iter().map(own_text_line).collect::<Vec<_>>());
            Ok((text, MyTextsOutput { kind, items, total_items, next_cursor }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_are_validated_and_scaled() {
        assert_eq!(rating_tenths(Scale::Scent, 7.5).unwrap(), 75);
        assert_eq!(rating_tenths(Scale::Scent, 0.0).unwrap(), 0);
        assert_eq!(rating_tenths(Scale::Bottle, 10.0).unwrap(), 100);
        for bad in [-0.5, 10.5, 7.3, f64::NAN, f64::INFINITY] {
            assert!(rating_tenths(Scale::Sillage, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn labels_resolve_case_insensitively_and_by_group() {
        let choice = |group: &str, label: &str, id: u64, voted: bool| ClassChoice {
            group: group.into(),
            label: label.into(),
            voted,
            group_id: 1,
            label_id: id,
        };
        let choices = vec![choice("Type", "Fresh", 17, true), choice("Type", "Woody", 19, false), choice("Occasion", "Night Out", 7, false)];
        assert_eq!(find_choice(&choices, " night out ").unwrap().label_id, 7);
        assert_eq!(find_choice(&choices, "Type: woody").unwrap().label_id, 19);
        let unknown = find_choice(&choices, "Rainy").unwrap_err();
        assert!(unknown.message.contains("Type (Fresh, Woody)"), "{}", unknown.message);
        assert!(find_choice(&choices, "Occasion: Fresh").is_err());
        let groups = group_votes(&choices);
        assert_eq!(groups[0], ClassVotes { group: "Type".into(), voted: vec!["Fresh".into()], options: vec!["Fresh".into(), "Woody".into()] });
        assert_eq!(votes_line(&groups), "Type: Fresh");
        assert!(same_text("a  b\nc", " a b c ") && !same_text("a b", "ab"));
    }
}
