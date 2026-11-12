//! Data models returned by the client. All derive `JsonSchema`: the doc comments
//! become the field descriptions of the MCP tool schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod social;
pub use social::*;

/// One page of a natively paginated listing (`?current_page=N`).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct Listing<T> {
    /// Items of this page, in site order.
    pub items: Vec<T>,
    /// 1-indexed page number that was fetched.
    pub page: u32,
    /// Total number of pages, when the site shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<u32>,
    /// Total number of items across all pages, when the site shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_items: Option<u32>,
}

/// A window of an offset-paginated list (reviews, statements).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct Slice<T> {
    /// Items of this window.
    pub items: Vec<T>,
    /// 0-indexed offset of the first item.
    pub offset: u32,
    /// Total number of items, when the site shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
}

/// Reference to a brand, perfumer or fragrance note.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct EntityRef {
    /// Stable identifier: the URL slug (e.g. `Dior`, `Francois_Demachy`, `Bergamot`).
    pub id: String,
    /// Numeric Parfumo identifier (b_id, pf_id or note id) when the page exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_id: Option<u64>,
    /// Display name.
    pub name: String,
    /// Absolute Parfumo URL.
    pub url: String,
}

/// A perfume as it appears in a listing (search, brand, ranking, similar).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct PerfumeSummary {
    /// Stable identifier: `Brand/Perfume` URL slug, usable with every perfume tool.
    pub id: String,
    /// Numeric Parfumo perfume id when the listing exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_id: Option<u64>,
    /// Perfume name.
    pub name: String,
    /// Brand name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Release year.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// Concentration or edition label shown next to the name (e.g. `Eau de Parfum`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<String>,
    /// Average scent rating out of 10, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    /// Number of ratings behind `rating`, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<u32>,
    /// Target gender (`men`, `women`, `unisex`), when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    /// Number of long reviews, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviews_count: Option<u32>,
    /// Counter shown by trend rankings (collection adds, wishes or wears over the last 30 days).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend_count: Option<u32>,
    /// Position in the ranking, when the listing is ranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    /// Absolute Parfumo URL of the perfume page.
    pub url: String,
    /// Bottle image URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// Fragrance pyramid of a perfume.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct NotePyramid {
    /// Top notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<String>,
    /// Heart notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heart: Vec<String>,
    /// Base notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base: Vec<String>,
    /// Notes listed without pyramid levels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flat: Vec<String>,
}

/// Community average on one rating scale.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct RatingScore {
    /// Scale: `scent`, `durability` (longevity), `sillage`, `bottle` or `pricing` (value for money).
    pub dimension: String,
    /// Average out of 10.
    pub average: f32,
    /// Number of votes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<u32>,
}

/// One slice of a community classification chart.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct ClassShare {
    /// Label (e.g. `Spring`, `Leisure`, `Fresh`, `Men`).
    pub label: String,
    /// Share of votes in percent.
    pub percent: f32,
}

/// One group of the community classification.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct ClassGroup {
    /// Group name as shown by Parfumo: `Type`, `Occasion`, `Season`, `Audience & Style`.
    pub group: String,
    /// Number of votes behind the chart, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<u32>,
    /// Shares in site order.
    pub shares: Vec<ClassShare>,
}

/// Full perfume record.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct PerfumeDetails {
    /// Stable identifier: `Brand/Perfume` URL slug.
    pub id: String,
    /// Numeric Parfumo perfume id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_id: Option<u64>,
    /// Perfume name.
    pub name: String,
    /// Brand.
    pub brand: EntityRef,
    /// Release year.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// Target gender as stated by Parfumo (`men`, `women`, `unisex`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    /// Concentration or edition (e.g. `Eau de Toilette`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<String>,
    /// Perfumers credited.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perfumers: Vec<EntityRef>,
    /// Fragrance notes.
    pub notes: NotePyramid,
    /// Main accords, strongest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accords: Vec<String>,
    /// Community averages (scent, durability, sillage, bottle, pricing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ratings: Vec<RatingScore>,
    /// Community classification (type, season, occasion, audience, style).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classification: Vec<ClassGroup>,
    /// Rank label shown on the page (e.g. `Ranked 30 in Men's Perfume`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking: Option<String>,
    /// "Interesting facts" paragraph, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<String>,
    /// Editorial description shown on the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Number of long reviews.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviews_count: Option<u32>,
    /// Number of statements (short reviews).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statements_count: Option<u32>,
    /// Bottle image URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Absolute Parfumo URL.
    pub url: String,
    /// Non-fatal problems: optional parts that could not be loaded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Ratings a member attached to a review or statement.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct MemberRatings {
    /// Scent rating out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scent: Option<f32>,
    /// Longevity rating out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durability: Option<f32>,
    /// Sillage rating out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sillage: Option<f32>,
    /// Bottle rating out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottle: Option<f32>,
    /// Value-for-money rating out of 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<f32>,
}

/// A long review written by a Parfumo member (third-party content).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Review {
    /// Numeric review id.
    pub id: u64,
    /// Absolute URL of the review.
    pub url: String,
    /// Member nickname.
    pub author: String,
    /// Absolute URL of the member profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// Publication date as shown by the site (ISO date when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Review title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Review text written by the member, HTML removed.
    pub text: String,
    /// True when only an excerpt could be retrieved.
    #[serde(default)]
    pub truncated: bool,
    /// Ratings given by the member.
    #[serde(default)]
    pub ratings: MemberRatings,
    /// Number of awards / "helpful" marks, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awards: Option<u32>,
    /// Number of comments, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
}

/// A statement (short review) written by a Parfumo member (third-party content).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Statement {
    /// Numeric statement id.
    pub id: u64,
    /// Absolute URL of the statement.
    pub url: String,
    /// Member nickname.
    pub author: String,
    /// Absolute URL of the member profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// Publication date or relative age as shown by the site.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Statement text written by the member, HTML removed.
    pub text: String,
    /// Ratings given by the member.
    #[serde(default)]
    pub ratings: MemberRatings,
    /// Number of awards, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awards: Option<u32>,
    /// Number of comments, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
}

/// A perfume members voted as similar to (or good to layer with) another one.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct SimilarPerfume {
    /// Numeric Parfumo perfume id.
    pub p_id: u64,
    /// Perfume name.
    pub name: String,
    /// Brand name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Concentration or edition label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<String>,
    /// Community consensus in percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
    /// Number of member votes (only for entries whose link was resolved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<u32>,
    /// `Brand/Perfume` id. The site lists similar perfumes without links: only the first
    /// entries are resolved; use `parfumo_search_perfumes` with name and brand for the others.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Absolute Parfumo URL (same restriction as `id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Bottle image URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// Similar perfumes, layering suggestions and reported dupes.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct SimilarPerfumes {
    /// `Brand/Perfume` id of the perfume the lists refer to.
    pub perfume_id: String,
    /// Perfumes voted as similar by members, strongest consensus first.
    pub similar: Vec<SimilarPerfume>,
    /// Perfumes members suggest layering with this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layering: Vec<SimilarPerfume>,
    /// Groups of Parfumo's dupes directory in which this perfume appears, as original or as dupe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dupes: Vec<DupeGroup>,
    /// Non-fatal problems: optional parts that could not be loaded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// One perfume reported as a dupe, with its vote count.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct DupeCandidate {
    /// The perfume reported as a dupe.
    pub perfume: PerfumeSummary,
    /// Number of members who reported it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<u32>,
}

/// One group of the dupes directory: an original and the perfumes reported as its dupes.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct DupeGroup {
    /// The original perfume.
    pub original: PerfumeSummary,
    /// Perfumes reported as dupes of the original.
    pub dupes: Vec<DupeCandidate>,
}

/// Options of `ParfumoClient::get_similar_perfumes_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimilarOptions {
    /// How many leading similar perfumes get their URL resolved (one extra request each).
    pub resolve_urls: usize,
    /// Also load layering suggestions (one extra request).
    pub include_layering: bool,
    /// Also search the dupes directory (one extra request).
    pub include_dupes: bool,
}

impl Default for SimilarOptions {
    fn default() -> Self {
        Self { resolve_urls: 5, include_layering: true, include_dupes: true }
    }
}

/// Brand record and one page of its perfumes.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct BrandPage {
    /// The brand.
    pub brand: EntityRef,
    /// Country of the brand, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Founding year, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub founded: Option<u16>,
    /// Perfumes of the brand (one native page).
    pub perfumes: Listing<PerfumeSummary>,
}

/// Perfumer record and one page of their creations.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct PerfumerPage {
    /// The perfumer.
    pub perfumer: EntityRef,
    /// Country of the perfumer, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Perfumes created (one native page).
    pub perfumes: Listing<PerfumeSummary>,
}

/// Fragrance note record and one page of perfumes containing it.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct NotePage {
    /// The note.
    pub note: EntityRef,
    /// True when this is a note category (group of notes) rather than a single note.
    pub is_category: bool,
    /// Categories this note belongs to (or related categories of a category).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<EntityRef>,
    /// Perfumes containing the note (one native page).
    pub perfumes: Listing<PerfumeSummary>,
}

/// A link found in a fetched page.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct PageLink {
    /// Link text.
    pub text: String,
    /// Absolute URL.
    pub url: String,
}

/// Cleaned text of an arbitrary Parfumo page.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct FetchedPage {
    /// Absolute URL that was fetched.
    pub url: String,
    /// Page title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Main text of the page, without scripts, styles or navigation. May contain third-party content.
    pub text: String,
    /// Links found in the main content (same site only, deduplicated, capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<PageLink>,
}

/// Target gender filter.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    /// Marketed to men.
    Men,
    /// Marketed to women.
    Women,
    /// Unisex.
    Unisex,
}

/// Sort order of a perfume search.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    /// Site default order.
    #[default]
    Default,
    /// Name A→Z.
    NameAsc,
    /// Name Z→A.
    NameDesc,
    /// Most recently added first.
    Newest,
    /// Most rated first.
    PopularityDesc,
    /// Least rated first.
    PopularityAsc,
    /// Newest release year first.
    YearDesc,
    /// Oldest release year first.
    YearAsc,
    /// Scent rating, site order "Scent (popular)".
    ScentDesc,
    /// Longevity, site order "Lasting (longest)".
    DurabilityDesc,
    /// Sillage, site order "Sillage (highest)".
    SillageDesc,
    /// Bottle rating, site order "Bottle (popular)".
    BottleDesc,
    /// Value for money, site order "Value for Money (high)".
    PricingDesc,
}

/// Perfume search criteria. Every filter is optional; at least one must be set.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct SearchQuery {
    /// Free text (perfume or brand name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Brand name; resolved to Parfumo's brand id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Perfumer name; resolved to Parfumo's perfumer id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfumer: Option<String>,
    /// Fragrance notes the perfume must contain (names; resolved to note ids).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Exact release year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// First release year of a range (inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_from: Option<u16>,
    /// Last release year of a range (inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_to: Option<u16>,
    /// Target genders; empty means all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genders: Vec<Gender>,
    /// Sort order.
    #[serde(default)]
    pub sort: SearchSort,
}

/// Sort order of reviews and statements.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSort {
    /// Most relevant first (site default).
    #[default]
    Relevance,
    /// Newest first.
    Newest,
    /// Oldest first.
    Oldest,
    /// Highest scent rating first.
    ScentDesc,
    /// Lowest scent rating first.
    ScentAsc,
    /// Most commented first.
    MostReplies,
}

/// Which ranking to read.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RankingKind {
    /// Top men's perfumes.
    TopMen,
    /// Top women's perfumes.
    TopWomen,
    /// Top unisex perfumes.
    TopUnisex,
    /// Recently added perfumes.
    New,
    /// Trending: most added to collections over the last 30 days.
    Trends,
    /// Trending: most wished over the last 30 days.
    TrendsWished,
    /// Trending: most worn over the last 30 days.
    TrendsWorn,
    /// Perfumes released in a given year.
    Year(u16),
}

// ---------------------------------------------------------------------------
// Account: collection and wears (jalon 2)
// ---------------------------------------------------------------------------

/// One list of the member's collection (ShelfControl `data-lists`).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct CollectionList {
    /// Numeric list id: 1 have, 2 had, 3 want (Wish List), 4 memo (Watch List), 5 tested,
    /// 6 filling (Decants), 7 mini (Miniatures); custom lists have a larger id.
    pub id: u64,
    /// Slug usable as `collection` argument: `have`, `had`, `want`, `memo`, `tested`,
    /// `filling`, `mini`, or the numeric id of a custom list.
    pub slug: String,
    /// Display name on Parfumo.
    pub name: String,
    /// Description of a custom list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// True when the list is hidden from other members.
    pub hidden: bool,
    /// Number of perfumes in the list.
    pub count: u32,
    /// True for a list created by the member.
    pub custom: bool,
    /// ShelfControl URL of the list (account page).
    pub url: String,
}

/// A custom ShelfControl column defined by the member.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct CustomColumn {
    /// Numeric column id.
    pub id: u64,
    /// Column name chosen by the member.
    pub name: String,
}

/// One bottle (entry) of a collection list, with the member's private fields.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct CollectionItem {
    /// Identifier of this entry in the list (Parfumo `wr_id`).
    pub entry_id: u64,
    /// Perfume identifier usable with every perfume tool (`Brand/Perfume`).
    pub perfume_id: String,
    /// Numeric Parfumo perfume id.
    pub p_id: u64,
    /// Perfume name, concentration included.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Absolute Parfumo URL of the perfume.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// `men`, `women` or `unisex`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<String>,
    /// The member's own ratings (0-10).
    #[serde(default, skip_serializing_if = "MemberRatings::is_empty")]
    pub my_ratings: MemberRatings,
    /// Community scent rating (0-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_rating: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_votes: Option<u32>,
    /// Number of bottles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    /// Bottle size as entered (millilitres, free text on the site).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottle_size: Option<String>,
    /// Millilitres left in the bottle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_ml: Option<f64>,
    /// Price paid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_paid: Option<f64>,
    /// Currency code of the price (e.g. `EUR`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The member's private note on this bottle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_note: Option<String>,
    /// Date the entry was added (`YYYY-MM-DD`, UTC). Set by the site, not editable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
    /// Number of logged wears.
    pub wear_count: u32,
    /// Date of the last logged wear (`YYYY-MM-DD`, UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_worn: Option<String>,
    /// Values of the member's custom columns, by column name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub custom: std::collections::BTreeMap<String, String>,
    /// True when the member wrote a statement / a review on this perfume.
    pub has_statement: bool,
    pub has_review: bool,
    /// Details resolved from the list dictionaries (omitted in compact listings).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perfumers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occasions: Vec<String>,
    /// Country of the brand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl MemberRatings {
    pub fn is_empty(&self) -> bool {
        self.scent.is_none() && self.durability.is_none() && self.sillage.is_none() && self.bottle.is_none() && self.pricing.is_none()
    }
}

/// One ShelfControl page: every list of the member plus the content of the active one.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Shelf {
    /// The list whose rows are in `items`.
    pub list: CollectionList,
    pub lists: Vec<CollectionList>,
    pub columns: Vec<CustomColumn>,
    pub items: Vec<CollectionItem>,
}

/// One logged wear ("perfume of the day").
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Wear {
    /// Identifier of the wear, to pass to parfumo_delete_wear.
    pub wear_id: u64,
    /// Perfume identifier usable with every perfume tool (`Brand/Perfume`).
    pub perfume_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    pub url: String,
    /// Local date and time of the wear as shown by Parfumo (`YYYY-MM-DDTHH:MM`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worn_at: Option<String>,
    /// The member's own note on this wear.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// A write request to Parfumo, as built before being sent (also shown by `dry_run`).
/// The session `csrf_key` is added at send time and never appears here.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct WriteCall {
    /// Always `POST`.
    pub method: String,
    /// Path on the Parfumo host.
    pub path: String,
    /// Form fields (`application/x-www-form-urlencoded`), without the CSRF token.
    pub form: std::collections::BTreeMap<String, String>,
    #[serde(skip)]
    #[schemars(skip)]
    pub expect: Expect,
}

/// What a successful answer of a write looks like (DISCOVERY §3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Expect {
    /// Success is an empty body (or anything): only a re-read proves the effect.
    #[default]
    Anything,
    /// A redirect (classic HTML form: fragrance preferences, blog). The target is returned.
    Redirect,
    /// JSON with `"success":true` (forum); `error` carries the refusal.
    Success,
    /// JSON with `"result":"success"` (Souk); `errors` / `error` carry the refusal.
    ResultSuccess,
    /// JSON whose `val` is empty (comments, wall); otherwise `val` is the refusal.
    CommentOk,
    /// A non-empty body (fragment, identifier).
    Body,
    /// The bare value `1`.
    One,
    /// JSON with `"status":"ok"`.
    StatusOk,
    /// JSON with `"ok":true`.
    OkTrue,
    /// JSON whose `status` is not `0` (reviews, statements). `{"status":0,"msg":…}` is a
    /// refusal by the site: the message is reported to the caller.
    Accepted,
}

// ---------------------------------------------------------------------------
// Community reads: blogs, Souk and wall (jalon 4)
// ---------------------------------------------------------------------------

/// Site-wide list of blog articles.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlogFeed {
    /// Latest articles of all members (8 per page).
    #[default]
    Latest,
    /// Latest articles of the "News" category.
    News,
    /// Most awarded articles of the week (single page).
    Top,
    /// Most discussed articles of the week (single page).
    Hot,
}

/// A blog article as it appears in a list (site-wide feed or a member's blog).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct BlogEntry {
    /// Stable identifier: `{nick}/{slug}`, as in the article URL.
    pub id: String,
    /// Absolute Parfumo URL of the article.
    pub url: String,
    /// Title, as written by the member.
    pub title: String,
    /// Nickname of the author.
    pub author: String,
    /// Absolute Parfumo URL of the author's profile.
    pub author_url: String,
    /// Publication date as shown by the site (relative, e.g. `2 days ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// First lines of the article, as shown in the list (member-written, cut by the site).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Number of comments; absent when the list shows none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
    /// Number of awards (trophies); absent when the list shows none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awards: Option<u32>,
    /// URL of the cover image, when the article has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// One blog article with its full text.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct BlogArticle {
    /// Stable identifier: `{nick}/{slug}`, as in the article URL.
    pub id: String,
    /// Numeric Parfumo id of the article (needed to comment on it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_id: Option<u64>,
    /// Absolute Parfumo URL of the article.
    pub url: String,
    /// Title, as written by the member.
    pub title: String,
    /// Nickname of the author.
    pub author: String,
    /// Absolute Parfumo URL of the author's profile.
    pub author_url: String,
    /// Numeric Parfumo id of the author, when the page carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<u64>,
    /// Name the member gave to their blog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blog_title: Option<String>,
    /// Publication date as shown by the site (e.g. `1 day ago - 10/11/2026`, day/month/year).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Full text of the article, paragraphs separated by blank lines (member-written).
    pub text: String,
    /// URLs of the images embedded in the text, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Number of comments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
    /// Number of awards (trophies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awards: Option<u32>,
    /// Comments rendered in the article page (the site may show only a part of a long thread).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comment_list: Vec<WallEntry>,
}

/// Section of the Souk, the members' marketplace.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SoukKind {
    /// A member offers a bottle or a sample (sale or swap).
    #[default]
    Offers,
    /// A member is looking for a bottle or a sample.
    Requests,
    /// A member splits a bottle between several participants ("Splits" on the site).
    Sharings,
}

/// A row of the details table of a Souk entry that has no dedicated field.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct SoukDetail {
    /// Label shown by the site, e.g. `Purchase Date`, `Minimum Content`, `Decant`.
    pub label: String,
    /// Value, mostly written by the member.
    pub value: String,
}

/// An entry of the Souk: an offer, a request or a split. Lists and search results only
/// carry the first fields; condition, size, price, shipping, note and photos are filled
/// when the entry page itself is read.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct SoukItem {
    /// Numeric Parfumo id of the entry (splits have their own numbering).
    pub id: u64,
    /// Offer, request or split.
    pub kind: SoukKind,
    /// Absolute Parfumo URL of the entry.
    pub url: String,
    /// Perfume name.
    pub perfume_name: String,
    /// Concentration label shown next to the name in lists (e.g. `Eau de Parfum`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<String>,
    /// Brand name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Perfume identifier usable with every perfume tool (`Brand/Perfume`); entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perfume_id: Option<String>,
    /// Absolute Parfumo URL of the perfume; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perfume_url: Option<String>,
    /// Nickname of the member who posted the entry.
    pub author: String,
    /// Absolute Parfumo URL of that member's profile.
    pub author_url: String,
    /// Numeric Parfumo id of that member; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<u64>,
    /// Country of the member, as named by the site (e.g. `Germany`); entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Country code of the member's flag (e.g. `de`), when a flag is shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    /// `Bottle` or `Sample` (search results and entry pages of offers and requests).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    /// Kind of deal chosen by the member ("Souk Option": e.g. `Purchase`, `Swap`); entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade: Option<String>,
    /// Preference shown under the headline (e.g. `Purchase preferred`); entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preference: Option<String>,
    /// Condition (e.g. `New`, `Second Hand`, `Any` for a request); entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Size or fill level: `25 ml`, `30 ml of 100 ml`, `16 ml left`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// What `size` measures: `Bottle size`, `Current content`, `Minimum Content`, `of 82 ml`; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_label: Option<String>,
    /// Price or price expectation, as written by the member; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    /// What the member would swap for, as written by the member; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_for: Option<String>,
    /// Shipping terms, as written by the member; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping: Option<String>,
    /// Free text of the member, line breaks preserved; entry page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Date as shown by the site (relative, e.g. `19 hours ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Other status lines of a split (e.g. `Closing in 1 month`, `Updated 4 minutes ago`; `Complete` in lists).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<String>,
    /// Number of participants of a split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participants: Option<u32>,
    /// True when the member attached at least one photo.
    pub has_photos: bool,
    /// URLs of the photos attached by the member; entry page only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<String>,
    /// Remaining rows of the details table (e.g. `Purchase Date`, `Minimum Content`, `Decant`); entry page only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<SoukDetail>,
}

/// An answer to a wall entry or to a comment.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct WallReply {
    /// Numeric Parfumo id of the reply.
    pub id: u64,
    /// Nickname of the member who wrote it.
    pub author: String,
    /// Absolute Parfumo URL of that member's profile.
    pub author_url: String,
    /// Date as shown by the site (relative, e.g. `3 years ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Text, as written by the member, line breaks preserved.
    pub text: String,
}

/// A message left by a member on a profile wall (also used for the comments of a blog article).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct WallEntry {
    /// Numeric Parfumo id of the entry (comment id).
    pub id: u64,
    /// Nickname of the member who wrote it.
    pub author: String,
    /// Absolute Parfumo URL of that member's profile.
    pub author_url: String,
    /// Date as shown by the site (relative, e.g. `18 days ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Text, as written by the member, line breaks preserved.
    pub text: String,
    /// Answers to this entry, oldest first as shown by the site.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<WallReply>,
}

/// One page of the wall of a member.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct WallPage {
    /// Nickname of the member who owns the wall.
    pub owner: String,
    /// Absolute Parfumo URL of the owner's profile.
    pub owner_url: String,
    /// Numeric Parfumo id of the owner (needed to write on the wall), when the page carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<u64>,
    /// Entries of this page, newest first, 20 per page.
    pub entries: Listing<WallEntry>,
}

// ---------------------------------------------------------------------------
// Account: ratings, classification, reviews and statements (jalon 3)
// ---------------------------------------------------------------------------

/// One label of the classification form of a perfume, with the member's vote.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ClassChoice {
    /// Group as shown by Parfumo: `Type`, `Style`, `Season` or `Occasion`.
    pub group: String,
    /// Label, e.g. `Fresh`, `Classic`, `Spring`, `Night Out`.
    pub label: String,
    /// True when the member voted for this label.
    pub voted: bool,
    /// Site identifiers of the group and of the label.
    #[serde(skip)]
    #[schemars(skip)]
    pub group_id: u64,
    #[serde(skip)]
    #[schemars(skip)]
    pub label_id: u64,
}

/// A perfume the member rated on one scale ("My Ratings" page).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct RatedPerfume {
    /// Perfume identifier usable with every perfume tool (`Brand/Perfume`).
    pub perfume_id: String,
    /// Numeric Parfumo perfume id.
    pub p_id: u64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    pub url: String,
    /// The member's rating on the scale of the page, out of 10.
    pub rating: f32,
    /// When it was rated, as shown by the site (relative, e.g. `3 days ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated: Option<String>,
}

/// A review or statement of the member, as listed on their own pages.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct OwnText {
    /// Numeric id of the review or statement.
    pub id: u64,
    /// Parfumo URL of the review or statement.
    pub url: String,
    /// Perfume identifier usable with every perfume tool (`Brand/Perfume`).
    pub perfume_id: String,
    pub perfume_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Age as shown by the site (reviews only, relative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Title (reviews only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    /// Ratings shown next to a review (0-10).
    #[serde(default, skip_serializing_if = "MemberRatings::is_empty")]
    pub ratings: MemberRatings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awards: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
}

/// The member's own review or statement on one perfume, as the edit form would load it.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct OwnDraft {
    /// Title (reviews only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Text; empty when the member wrote nothing on this perfume.
    pub text: String,
}

/// What the delete button of a review or statement form carries. `h` is a site token:
/// it is sent, never shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteHandle {
    pub id: u64,
    pub p_id: u64,
    pub h: String,
}

/// Review or statement.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    /// Long review (title and text).
    #[default]
    Review,
    /// Statement: short review of 140 characters at most.
    Statement,
}

impl MemberRatings {
    /// Rating on a scale named as the site does (`scent`, `durability`, `sillage`, `bottle`, `pricing`).
    pub fn get(&self, scale: &str) -> Option<f32> {
        match scale {
            "scent" => self.scent,
            "durability" => self.durability,
            "sillage" => self.sillage,
            "bottle" => self.bottle,
            "pricing" => self.pricing,
            _ => None,
        }
    }

    pub fn set(&mut self, scale: &str, value: Option<f32>) {
        match scale {
            "scent" => self.scent = value,
            "durability" => self.durability = value,
            "sillage" => self.sillage = value,
            "bottle" => self.bottle = value,
            "pricing" => self.pricing = value,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Forum and communities (jalon 4, read only)
// ---------------------------------------------------------------------------

/// A category of the Parfumo forum.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ForumCategory {
    /// Stable identifier: the URL slug (e.g. `perfumes-brands`), usable to list the topics.
    pub slug: String,
    /// Numeric forum id used by the site's own calls (needed to open a topic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forum_id: Option<u64>,
    /// Category name.
    pub name: String,
    /// Short description shown under the name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Section of the forum home the category is listed under (e.g. `The World of Fragrances`).
    /// Only filled by the list of categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Number of topics in the category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<u32>,
    /// Number of topics active today. Only filled by the list of categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics_today: Option<u32>,
    /// Absolute Parfumo URL of the category.
    pub url: String,
}

/// A topic (thread) as listed in a forum category.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ForumTopic {
    /// Stable identifier: the URL slug of the topic, usable to read the thread.
    pub slug: String,
    /// Topic title, written by a member.
    pub title: String,
    /// Absolute Parfumo URL of the first page of the topic.
    pub url: String,
    /// Nickname of the member who opened the topic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Absolute URL of the author's profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// Number of replies (posts after the first one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replies: Option<u32>,
    /// Number of pages of the topic (25 posts per page), when it has more than one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    /// Numeric id of the most recent post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post_id: Option<u64>,
    /// Absolute URL of the most recent post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post_url: Option<String>,
    /// Nickname of the author of the most recent post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post_author: Option<String>,
    /// Date of the most recent post, as shown by the site (e.g. `8 hours ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post_date: Option<String>,
    /// True for a sticky topic, pinned above the others on the first page.
    pub pinned: bool,
    /// True when the topic is locked (no new reply).
    pub locked: bool,
}

/// A forum category and one page of its topics (50 per page).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct ForumTopicsPage {
    /// The category.
    pub category: ForumCategory,
    /// Topics of this page: sticky topics first (page 1 only), then by latest activity.
    pub topics: Listing<ForumTopic>,
}

/// Head of a forum topic, as read on one of its pages.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ForumTopicHead {
    /// Stable identifier: the URL slug of the topic.
    pub slug: String,
    /// Numeric topic id used by the site's own calls (needed to reply).
    pub topic_id: u64,
    /// Numeric id of the category the topic belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forum_id: Option<u64>,
    /// Topic title, written by a member.
    pub title: String,
    /// Absolute Parfumo URL of the first page of the topic.
    pub url: String,
    /// URL slug of the category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_slug: Option<String>,
    /// Name of the category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_name: Option<String>,
    /// Absolute URL of the category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_url: Option<String>,
}

/// One post of a forum topic.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ForumPost {
    /// Stable identifier: numeric post id.
    pub id: u64,
    /// Absolute Parfumo URL of the post (opens the page of the thread that holds it).
    pub url: String,
    /// Nickname of the author.
    pub author: String,
    /// Absolute URL of the author's profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// Date as shown by the site (relative, e.g. `1 year ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Publication date and time in ISO 8601, when the page carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_iso: Option<String>,
    /// Title of the post: only the opening post of a topic has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Full text written by the member, line breaks preserved. A quoted post stays inline:
    /// the quoted member's nickname on one line, then the quoted text.
    pub text: String,
    /// Number of members who thanked the author for this post (heart counter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thanks: Option<u32>,
}

/// A forum topic and one page of its posts (25 per page).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct ForumThread {
    /// The topic.
    pub topic: ForumTopicHead,
    /// Posts of this page, oldest first. `page` is the page actually shown, which for a
    /// post URL is the page holding that post.
    pub posts: Listing<ForumPost>,
}

/// One result of the forum search: a post matching the text.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct ForumSearchHit {
    /// Stable identifier: numeric id of the matching post.
    pub post_id: u64,
    /// Absolute Parfumo URL of the post, usable to read the thread around it.
    pub url: String,
    /// Title of the topic the post belongs to, written by a member.
    pub title: String,
    /// Nickname of the author of the post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Absolute URL of the author's profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// URL slug of the category of the topic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_slug: Option<String>,
    /// Name of the category of the topic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_name: Option<String>,
    /// Date of the post, as shown by the site (e.g. `5 days ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Excerpt of the post around the matching words, written by a member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A community (group of members around a theme).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Community {
    /// Stable identifier: the URL slug (e.g. `scent-of-the-day`).
    pub slug: String,
    /// Numeric group id used by the site's own calls. Only filled by the community page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u64>,
    /// Community name, chosen by the member who created it.
    pub name: String,
    /// One-line tagline, written by a member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    /// Description, written by a member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Number of members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<u32>,
    /// Number of posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<u32>,
    /// Access badge as shown by the site (`Public`, `Invite-only`), when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    /// Tags of the community, without the leading `#`. Only filled by the community page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Absolute Parfumo URL of the community.
    pub url: String,
}

/// Short reference to a community.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct CommunityRef {
    /// Stable identifier: the URL slug of the community.
    pub slug: String,
    /// Community name.
    pub name: String,
    /// Absolute Parfumo URL of the community.
    pub url: String,
}

/// A post of a community.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct CommunityPost {
    /// Stable identifier: the URL slug of the post inside its community.
    pub slug: String,
    /// Numeric post id used by the site's own calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// URL slug of the community the post belongs to.
    pub community_slug: String,
    /// Absolute Parfumo URL of the post.
    pub url: String,
    /// Title, written by a member. Optional in a community feed; on the post page the site
    /// shows the beginning of the text when the member gave no title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Nickname of the author.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Absolute URL of the author's profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    /// Date as shown by the site (relative, e.g. `2 hours ago`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Publication date and time in ISO 8601. Only filled by the post page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_iso: Option<String>,
    /// Text written by the member, line breaks preserved: the full text on the post page,
    /// the text as shown in the feed (possibly shortened) in a community listing.
    pub text: String,
    /// Flair (category label inside the community), when the post has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    /// Perfumes attached to the post, each as `Name | Brand`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perfumes: Vec<String>,
    /// Vote score of the post (upvotes minus downvotes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<i32>,
    /// Number of comments. Only filled by the post page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u32>,
}

/// A community and one page of its posts (20 per page, newest first).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct CommunityPage {
    /// The community.
    pub community: Community,
    /// Posts of this page.
    pub posts: Listing<CommunityPost>,
}

/// One community post with its full text. The comments are not included: the public page
/// does not contain them (see `warnings`); `post.comments` gives their number.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct CommunityPostPage {
    /// The community the post belongs to.
    pub community: CommunityRef,
    /// The post.
    pub post: CommunityPost,
    /// What could not be read from the public page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
