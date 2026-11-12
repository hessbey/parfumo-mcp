//! Account tools of jalon 2: collection (section 7.2) and wears (section 7.3).
//!
//! All of them act on the Parfumo account linked by the calling user, through that user's
//! own session. Reads come from ShelfControl (one request per list). Every write tool takes
//! `dry_run`, only ever touches entries found in the user's own lists, and re-reads the
//! page afterwards to report the state Parfumo really holds (section 8).
//!
//! `parfumo_log_wear` is a public write (the perfume shows on the member's profile): it
//! stays a dry run unless the operator sets `PARFUMO_MCP_PUBLIC_WRITES`.

use std::collections::BTreeMap;

use parfumo_client::{CollectionItem, CollectionList, CustomColumn, ParfumoClient, Shelf, Wear, WriteCall, calls, find_list, parse};
use rmcp::{
    ErrorData,
    handler::server::{common::schema_for_output, wrapper::Parameters},
    model::CallToolResult,
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    caller::Caller,
    errors::{ErrorCode, ToolError},
    paging::{self, Cursor, MAX_STRUCTURED_CHARS},
    tools::{Clients, ParfumoTools},
};

const DEFAULT_COLLECTION: &str = "have";
const LIST_HAVE: u64 = 1;
const LIST_HAD: u64 = 2;
const LAST_FIXED_LIST: u64 = 7;
const LIST_NAME_MAX: usize = 40;
const LIST_NOTES_MAX: usize = 120;
const WEAR_COMMENT_MAX: usize = 255;
const NOTE_MAX: usize = 5_000;
/// Calendar months read by one `parfumo_list_wears` call (one request each).
const MAX_WEAR_MONTHS: u32 = 12;
const CSV_CHUNK_CHARS: usize = 18_000;
const STATS_TOP_DEFAULT: u32 = 10;
const STATS_TOP_MAX: u32 = 50;

// ---- parameters ----------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct NoParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetCollectionParams {
    /// Which list: `have` (default), `had`, `want` (Wish List), `memo` (Watch List), `tested`, `filling` (Decants), `mini` (Miniatures), or the id or name of a custom list as returned by parfumo_list_collections.
    #[serde(default)]
    pub collection: Option<String>,
    /// Also return perfumers, accords, notes, seasons and occasions of each perfume (larger answer). Default false.
    #[serde(default)]
    pub include_details: bool,
    /// Maximum number of items to return (default 25, maximum 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque pagination cursor: the `next_cursor` of the previous response.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddToCollectionParams {
    /// Perfume: the `id` returned by other tools (e.g. "Dior/Sauvage_Eau_de_Toilette"), its Parfumo URL, or its numeric `p_id`.
    pub perfume: String,
    /// Target list (default `have`). See parfumo_list_collections.
    #[serde(default)]
    pub collection: Option<String>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveCollectionItemParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`. It must be in the `from` list.
    pub perfume: String,
    /// Source list (slug, id or name).
    pub from: String,
    /// Destination list (slug, id or name).
    pub to: String,
    /// When true the perfume is copied and stays in the source list. Default false (move).
    #[serde(default)]
    pub copy: bool,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCollectionItemParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`. It must be in the list.
    pub perfume: String,
    /// List holding the bottle (default `have`).
    #[serde(default)]
    pub collection: Option<String>,
    /// Number of bottles.
    #[serde(default)]
    pub quantity: Option<u32>,
    /// Bottle size in millilitres, e.g. "100".
    #[serde(default)]
    pub bottle_size: Option<String>,
    /// Millilitres left in the bottle.
    #[serde(default)]
    pub remaining_ml: Option<f64>,
    /// Price paid. Give `currency` too unless the bottle already has one.
    #[serde(default)]
    pub price_paid: Option<f64>,
    /// Currency code of the price, e.g. "EUR".
    #[serde(default)]
    pub currency: Option<String>,
    /// Private note on this bottle. An empty string deletes the note.
    #[serde(default)]
    pub private_note: Option<String>,
    /// Values of custom ShelfControl columns, by column name (see parfumo_list_collections). Checkbox columns take "1" or "0", date columns "YYYY-MM-DD".
    #[serde(default)]
    pub custom: Option<BTreeMap<String, String>>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveFromCollectionParams {
    /// Perfume: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// List to remove it from (default `have`).
    #[serde(default)]
    pub collection: Option<String>,
    /// Parfumo moves a perfume removed from `have` into `had`. Set true to remove it from `had` as well, so that it disappears entirely. Default false.
    #[serde(default)]
    pub also_from_had: bool,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManageAction {
    /// Create a custom list (`name` required).
    Create,
    /// Change the name and/or description of a custom list.
    Rename,
    /// Hide a list from other members.
    Hide,
    /// Make a hidden list visible again.
    Unhide,
    /// Delete a custom list AND every entry in it. Cannot be undone.
    Delete,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ManageCustomCollectionParams {
    /// What to do.
    pub action: ManageAction,
    /// The list to act on (id or name). Not used by `create`.
    #[serde(default)]
    pub collection: Option<String>,
    /// Name of the list (40 characters at most). Required by `create`.
    #[serde(default)]
    pub name: Option<String>,
    /// Description of the list (120 characters at most).
    #[serde(default)]
    pub notes: Option<String>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Every field of every bottle as JSON objects.
    #[default]
    Json,
    /// CSV built by this server with every field, custom columns included.
    Csv,
    /// The CSV file Parfumo itself generates (10 columns only).
    CsvSite,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportCollectionParams {
    /// Which list to export (default `have`).
    #[serde(default)]
    pub collection: Option<String>,
    /// `json` (default), `csv` (all fields) or `csv_site` (Parfumo's own 10-column export).
    #[serde(default)]
    pub format: Option<ExportFormat>,
    /// Opaque cursor to continue a long export: the `next_cursor` of the previous response.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CollectionStatsParams {
    /// Which list to analyse (default `have`).
    #[serde(default)]
    pub collection: Option<String>,
    /// Length of each ranking (default 10, maximum 50).
    #[serde(default)]
    pub top: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LogWearParams {
    /// Perfume worn: `id`, Parfumo URL or numeric `p_id`.
    pub perfume: String,
    /// When it was worn: "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM", not in the future. Default: now.
    #[serde(default)]
    pub date: Option<String>,
    /// Personal note on this wear (255 characters at most).
    #[serde(default)]
    pub comment: Option<String>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWearsParams {
    /// First day of the period, "YYYY-MM-DD". Default: the first day of the month two months before `to`.
    #[serde(default)]
    pub from: Option<String>,
    /// Last day of the period, "YYYY-MM-DD". Default: today (UTC). A period spans 12 calendar months at most.
    #[serde(default)]
    pub to: Option<String>,
    /// Maximum number of wears to return (default 25, maximum 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque pagination cursor: the `next_cursor` of the previous response.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteWearParams {
    /// Identifier of the wear, as returned by parfumo_list_wears (`wear_id`).
    pub wear_id: u64,
    /// When true, nothing is sent: the answer shows the request that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

// ---- outputs -------------------------------------------------------------------------------

#[derive(Debug, Serialize, JsonSchema)]
pub struct CollectionsOutput {
    /// Every list of the account, fixed lists first.
    pub collections: Vec<CollectionList>,
    /// Custom ShelfControl columns usable with parfumo_update_collection_item.
    pub custom_columns: Vec<CustomColumn>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CollectionOutput {
    pub collection: CollectionList,
    /// Bottles of the list, in Parfumo's order.
    pub items: Vec<CollectionItem>,
    /// Number of bottles in the list.
    pub total_items: u32,
    /// Pass as `cursor` to get the following items. Absent when there is nothing more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result of a collection write.
#[derive(Debug, Serialize, JsonSchema, Default)]
pub struct CollectionWriteOutput {
    /// True when nothing was sent to Parfumo.
    pub dry_run: bool,
    /// True when at least one write request was sent.
    pub executed: bool,
    /// Outcome: `added`, `already_present`, `moved`, `copied`, `already_in_target`, `updated`, `removed`, `absent`, `created`, `already_exists`, `renamed`, `hidden`, `visible`, `unchanged`, `deleted`, or `dry_run`.
    pub status: String,
    /// The write requests sent, or that would be sent (CSRF token omitted).
    pub planned_requests: Vec<WriteCall>,
    /// The bottle as Parfumo holds it after the call (re-read from the site).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<CollectionItem>,
    /// The list concerned, with its count after the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionList>,
    /// Side effects and things to know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportOutput {
    pub collection: CollectionList,
    /// `json`, `csv` or `csv_site`.
    pub format: String,
    /// Number of bottles in the list.
    pub total_items: u32,
    /// 0-indexed position of the first row of this part.
    pub offset: u32,
    /// Number of rows in this part.
    pub returned: u32,
    /// Rows as JSON objects (format `json`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<CollectionItem>>,
    /// CSV text (formats `csv` and `csv_site`). The header line is only in the first part: concatenate the parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv: Option<String>,
    /// Pass as `cursor` to get the next part. Absent when the export is complete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct Count {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct CurrencyValue {
    /// Currency code, or `unknown` when a price has no currency.
    pub currency: String,
    /// Sum of the prices paid (price × nothing: the site stores one price per entry).
    pub total_paid: f64,
    /// Number of entries with a price in this currency.
    pub priced_items: u32,
    pub average_paid: f64,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct WornPerfume {
    /// Perfume identifier usable with every perfume tool.
    pub perfume_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    pub wear_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_worn: Option<String>,
}

/// Statistics computed by this server from one list.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatsOutput {
    pub collection: CollectionList,
    /// Number of entries (perfumes) in the list.
    pub total_items: u32,
    /// Sum of the quantities (bottles).
    pub total_bottles: u32,
    /// Number of distinct brands.
    pub distinct_brands: u32,
    /// Most frequent brands.
    pub brands: Vec<Count>,
    /// Dominant fragrance notes across the list.
    pub notes: Vec<Count>,
    /// Dominant accords.
    pub accords: Vec<Count>,
    pub perfumers: Vec<Count>,
    pub genders: Vec<Count>,
    pub concentrations: Vec<Count>,
    /// Release decades, e.g. `2010s`.
    pub decades: Vec<Count>,
    /// Seasons the community associates with the perfumes.
    pub seasons: Vec<Count>,
    pub occasions: Vec<Count>,
    /// Money spent, per currency (amounts in different currencies are never added up).
    pub value: Vec<CurrencyValue>,
    /// Entries without a recorded price.
    pub items_without_price: u32,
    /// Millilitres left, summed over the entries where it is recorded.
    pub remaining_ml_total: f64,
    /// Logged wears, summed over the list.
    pub total_wears: u32,
    /// Number of perfumes of the list never logged as worn.
    pub never_worn_count: u32,
    /// Perfumes never worn (first ones, in list order).
    pub never_worn: Vec<WornPerfume>,
    pub most_worn: Vec<WornPerfume>,
    /// Number of perfumes the member rated for scent, and the average of those ratings (0-10).
    pub rated_items: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_average_scent_rating: Option<f32>,
    /// Average community scent rating of the list (0-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_average_rating: Option<f32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WearsOutput {
    /// Period read, inclusive.
    pub from: String,
    pub to: String,
    /// Wears, most recent first.
    pub wears: Vec<Wear>,
    /// Number of wears in the period.
    pub total_in_period: u32,
    /// Pass as `cursor` to get the following wears. Absent when there is nothing more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result of a wear write.
#[derive(Debug, Serialize, JsonSchema, Default)]
pub struct WearWriteOutput {
    /// True when nothing was sent to Parfumo.
    pub dry_run: bool,
    /// True when at least one write request was sent.
    pub executed: bool,
    /// Outcome: `logged`, `logged_unverified`, `deleted`, `absent`, `still_listed` or `dry_run`.
    pub status: String,
    /// Whether this server is allowed to send publicly visible writes (operator setting).
    pub public_writes_enabled: bool,
    /// The write requests sent, or that would be sent (CSRF token omitted).
    pub planned_requests: Vec<WriteCall>,
    /// The wear as Parfumo lists it after the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wear: Option<Wear>,
    /// Side effects and things to know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

// ---- helpers -------------------------------------------------------------------------------

enum PerfumeKey {
    PId(u64),
    Slug(String),
}

fn perfume_key(client: &ParfumoClient, reference: &str) -> Result<PerfumeKey, ToolError> {
    let r = reference.trim();
    if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()) {
        return r.parse().map(PerfumeKey::PId).map_err(|_| ToolError::invalid("`perfume`: numeric id out of range."));
    }
    client.perfume_ref_id(r).map(PerfumeKey::Slug).map_err(|e| ToolError::invalid(e.to_string()))
}

fn find_item<'a>(shelf: &'a Shelf, key: &PerfumeKey) -> Option<&'a CollectionItem> {
    shelf.items.iter().find(|i| match key {
        PerfumeKey::PId(id) => i.p_id == *id,
        PerfumeKey::Slug(slug) => i.perfume_id.eq_ignore_ascii_case(slug),
    })
}

fn not_in_list(perfume: &str, list: &CollectionList) -> ToolError {
    ToolError::new(
        ErrorCode::NotFound,
        format!("'{perfume}' is not in the list '{}' (id {}). Use parfumo_get_collection to see what the list holds.", list.name, list.id),
    )
}

fn list_ref<'a>(lists: &'a [CollectionList], reference: &str) -> Result<&'a CollectionList, ToolError> {
    find_list(lists, reference).ok_or_else(|| {
        ToolError::new(
            ErrorCode::NotFound,
            format!("No collection '{}' on this account. Use parfumo_list_collections to see the lists.", reference.trim()),
        )
    })
}

fn compact(mut item: CollectionItem) -> CollectionItem {
    item.perfumers.clear();
    item.accords.clear();
    item.notes.clear();
    item.seasons.clear();
    item.occasions.clear();
    item.country = None;
    item
}

/// `85`, `12.5`: numbers as the site's own form sends them.
fn plain_number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn item_line(i: &CollectionItem) -> String {
    let mut s = i.name.clone();
    if let Some(b) = &i.brand {
        s.push_str(&format!(" — {b}"));
    }
    if let Some(y) = i.year {
        s.push_str(&format!(" ({y})"));
    }
    if let Some(q) = i.quantity.filter(|q| *q != 1) {
        s.push_str(&format!(" · ×{q}"));
    }
    if let Some(b) = &i.bottle_size {
        s.push_str(&format!(" · {b} ml"));
    }
    if let Some(ml) = i.remaining_ml {
        s.push_str(&format!(" · {} ml left", plain_number(ml)));
    }
    if let Some(p) = i.price_paid {
        s.push_str(&format!(" · paid {} {}", plain_number(p), i.currency.as_deref().unwrap_or("")));
    }
    if i.wear_count > 0 {
        s.push_str(&format!(" · worn {}×", i.wear_count));
    }
    if let Some(r) = i.my_ratings.scent {
        s.push_str(&format!(" · my scent rating {r}"));
    }
    s.push_str(&format!(" · id: {}", i.perfume_id));
    s
}

fn list_line(l: &CollectionList) -> String {
    format!(
        "{} — {} perfume(s) · collection: {}{}{}",
        l.name,
        l.count,
        l.slug,
        if l.custom { " (custom)" } else { "" },
        if l.hidden { " (hidden)" } else { "" }
    )
}

fn wear_line(w: &Wear) -> String {
    format!(
        "{} — {}{} · wear_id: {} · id: {}",
        w.worn_at.as_deref().unwrap_or("undated"),
        w.name,
        w.brand.as_deref().map(|b| format!(" ({b})")).unwrap_or_default(),
        w.wear_id,
        w.perfume_id
    )
}

pub(crate) fn planned_text(calls: &[WriteCall]) -> String {
    calls
        .iter()
        .map(|c| format!("POST {} {}", c.path, c.form.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_text(out: &CollectionWriteOutput) -> String {
    let mut text = if out.dry_run {
        format!("Dry run: nothing was sent to Parfumo. Requests that would be made:\n{}", planned_text(&out.planned_requests))
    } else {
        format!("Status: {}.", out.status)
    };
    if let Some(i) = &out.item {
        text.push_str(&format!("\n{}", item_line(i)));
    }
    if let Some(l) = &out.collection {
        text.push_str(&format!("\n{}", list_line(l)));
    }
    for n in &out.notes {
        text.push_str(&format!("\nNote: {n}"));
    }
    text
}

fn wear_write_text(out: &WearWriteOutput) -> String {
    let mut text = if out.dry_run {
        format!("Dry run: nothing was sent to Parfumo. Requests that would be made:\n{}", planned_text(&out.planned_requests))
    } else {
        format!("Status: {}.", out.status)
    };
    if let Some(w) = &out.wear {
        text.push_str(&format!("\n{}", wear_line(w)));
    }
    for n in &out.notes {
        text.push_str(&format!("\nNote: {n}"));
    }
    text
}

pub(crate) fn chars_at_most(what: &str, value: &str, max: usize) -> Result<(), ToolError> {
    if value.chars().count() > max {
        return Err(ToolError::invalid(format!("`{what}` is limited to {max} characters by Parfumo.")));
    }
    Ok(())
}

// ---- dates ---------------------------------------------------------------------------------

type Ymd = (i64, u32, u32);

fn today_utc() -> Ymd {
    parse::civil_from_days(crate::db::now().div_euclid(86_400))
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

fn parse_ymd(what: &str, s: &str) -> Result<Ymd, ToolError> {
    let bad = || ToolError::invalid(format!("`{what}` must be a date written YYYY-MM-DD."));
    let parts: Vec<&str> = s.trim().split('-').collect();
    let [y, m, d] = parts.as_slice() else { return Err(bad()) };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(bad());
    }
    let (y, m, d) = (y.parse::<i64>().map_err(|_| bad())?, m.parse::<u32>().map_err(|_| bad())?, d.parse::<u32>().map_err(|_| bad())?);
    if !(2000..=2100).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return Err(bad());
    }
    Ok((y, m, d))
}

fn ymd_text((y, m, d): Ymd) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` → the `datetime-local` value Parfumo expects.
/// A date alone is taken at noon.
fn parse_wear_date(s: &str, today: Ymd) -> Result<String, ToolError> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let ymd = parse_ymd("date", date)?;
    if ymd > today {
        return Err(ToolError::invalid("`date` cannot be in the future."));
    }
    let (hour, minute) = match time {
        None => (12, 0),
        Some(t) => {
            let bad = || ToolError::invalid("`date` time must be written HH:MM (24-hour clock).");
            let (h, m) = t.split_once(':').ok_or_else(bad)?;
            let m = m.get(..2).ok_or_else(bad)?;
            let (h, m) = (h.parse::<u32>().map_err(|_| bad())?, m.parse::<u32>().map_err(|_| bad())?);
            if h > 23 || m > 59 {
                return Err(bad());
            }
            (h, m)
        }
    };
    Ok(format!("{}T{hour:02}:{minute:02}", ymd_text(ymd)))
}

/// Months from `to` back to `from`, newest first.
fn months_between(from: Ymd, to: Ymd) -> Vec<(i64, u32)> {
    let mut out = Vec::new();
    let (mut y, mut m) = (to.0, to.1);
    while (y, m) >= (from.0, from.1) {
        out.push((y, m));
        if m == 1 {
            (y, m) = (y - 1, 12);
        } else {
            m -= 1;
        }
    }
    out
}

// ---- export and statistics -----------------------------------------------------------------

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn csv_line(fields: &[String]) -> String {
    fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",")
}

const CSV_COLUMNS: &[&str] = &[
    "Perfume",
    "Brand",
    "Year",
    "Gender",
    "Concentration",
    "Country",
    "Perfumers",
    "Accords",
    "Notes",
    "Seasons",
    "Occasions",
    "My Scent",
    "My Longevity",
    "My Sillage",
    "My Bottle",
    "My Value",
    "Community Rating",
    "Community Votes",
    "Quantity",
    "Bottle Size",
    "Remaining ml",
    "Paid",
    "Currency",
    "Private Note",
    "Date Added",
    "Last Worn",
    "Wear Count",
    "Statement",
    "Review",
];

pub fn csv_header(columns: &[CustomColumn]) -> String {
    let mut fields: Vec<String> = CSV_COLUMNS.iter().map(|c| c.to_string()).collect();
    fields.extend(columns.iter().map(|c| c.name.clone()));
    fields.extend(["Perfume Id", "Parfumo p_id", "Entry Id", "URL"].map(String::from));
    csv_line(&fields)
}

pub fn csv_row(i: &CollectionItem, columns: &[CustomColumn]) -> String {
    let opt = |v: Option<String>| v.unwrap_or_default();
    let num = |v: Option<f32>| v.map(|v| plain_number(f64::from(v))).unwrap_or_default();
    let yes = |b: bool| if b { "yes" } else { "" }.to_string();
    let mut fields = vec![
        i.name.clone(),
        opt(i.brand.clone()),
        opt(i.year.map(|y| y.to_string())),
        opt(i.gender.clone()),
        opt(i.concentration.clone()),
        opt(i.country.clone()),
        i.perfumers.join("; "),
        i.accords.join("; "),
        i.notes.join("; "),
        i.seasons.join("; "),
        i.occasions.join("; "),
        num(i.my_ratings.scent),
        num(i.my_ratings.durability),
        num(i.my_ratings.sillage),
        num(i.my_ratings.bottle),
        num(i.my_ratings.pricing),
        num(i.community_rating),
        opt(i.community_votes.map(|v| v.to_string())),
        opt(i.quantity.map(|v| v.to_string())),
        opt(i.bottle_size.clone()),
        opt(i.remaining_ml.map(plain_number)),
        opt(i.price_paid.map(plain_number)),
        opt(i.currency.clone()),
        opt(i.private_note.clone()),
        opt(i.added.clone()),
        opt(i.last_worn.clone()),
        i.wear_count.to_string(),
        yes(i.has_statement),
        yes(i.has_review),
    ];
    fields.extend(columns.iter().map(|c| i.custom.get(&c.name).cloned().unwrap_or_default()));
    fields.extend([i.perfume_id.clone(), i.p_id.to_string(), i.entry_id.to_string(), i.url.clone()]);
    csv_line(&fields)
}

/// Takes lines from `offset` while they fit in the chunk budget (always at least one).
fn take_lines(lines: &[String], offset: usize, budget: usize) -> (String, usize) {
    let mut out = String::new();
    let mut taken = 0;
    for line in lines.iter().skip(offset) {
        if taken > 0 && out.chars().count() + line.chars().count() + 1 > budget {
            break;
        }
        out.push_str(line);
        out.push('\n');
        taken += 1;
    }
    (out, taken)
}

fn top_counts<'a>(values: impl Iterator<Item = &'a str>, top: usize) -> Vec<Count> {
    let mut map: BTreeMap<&str, u32> = BTreeMap::new();
    for v in values.filter(|v| !v.trim().is_empty()) {
        *map.entry(v).or_default() += 1;
    }
    let mut counts: Vec<Count> = map.into_iter().map(|(name, count)| Count { name: name.to_string(), count }).collect();
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    counts.truncate(top);
    counts
}

fn worn(i: &CollectionItem) -> WornPerfume {
    WornPerfume {
        perfume_id: i.perfume_id.clone(),
        name: i.name.clone(),
        brand: i.brand.clone(),
        wear_count: i.wear_count,
        last_worn: i.last_worn.clone(),
    }
}

pub fn compute_stats(collection: CollectionList, items: &[CollectionItem], top: usize) -> StatsOutput {
    let mean = |values: Vec<f32>| (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32);
    let decades: Vec<String> = items.iter().filter_map(|i| i.year).map(|y| format!("{}s", y / 10 * 10)).collect();

    let mut by_currency: BTreeMap<String, (f64, u32)> = BTreeMap::new();
    for i in items {
        if let Some(p) = i.price_paid {
            let e = by_currency.entry(i.currency.clone().unwrap_or_else(|| "unknown".into())).or_default();
            e.0 += p;
            e.1 += 1;
        }
    }
    let value = by_currency
        .into_iter()
        .map(|(currency, (total, n))| CurrencyValue {
            currency,
            total_paid: (total * 100.0).round() / 100.0,
            priced_items: n,
            average_paid: (total / f64::from(n) * 100.0).round() / 100.0,
        })
        .collect();

    let mut most_worn: Vec<&CollectionItem> = items.iter().filter(|i| i.wear_count > 0).collect();
    most_worn.sort_by(|a, b| b.wear_count.cmp(&a.wear_count).then_with(|| a.name.cmp(&b.name)));
    let never: Vec<&CollectionItem> = items.iter().filter(|i| i.wear_count == 0).collect();
    let my_scent: Vec<f32> = items.iter().filter_map(|i| i.my_ratings.scent).collect();

    StatsOutput {
        collection,
        total_items: items.len() as u32,
        total_bottles: items.iter().map(|i| i.quantity.unwrap_or(1)).sum(),
        distinct_brands: items.iter().filter_map(|i| i.brand.as_deref()).collect::<std::collections::BTreeSet<_>>().len() as u32,
        brands: top_counts(items.iter().filter_map(|i| i.brand.as_deref()), top),
        notes: top_counts(items.iter().flat_map(|i| i.notes.iter().map(String::as_str)), top),
        accords: top_counts(items.iter().flat_map(|i| i.accords.iter().map(String::as_str)), top),
        perfumers: top_counts(items.iter().flat_map(|i| i.perfumers.iter().map(String::as_str)), top),
        genders: top_counts(items.iter().filter_map(|i| i.gender.as_deref()), top),
        concentrations: top_counts(items.iter().filter_map(|i| i.concentration.as_deref()), top),
        decades: top_counts(decades.iter().map(String::as_str), top),
        seasons: top_counts(items.iter().flat_map(|i| i.seasons.iter().map(String::as_str)), top),
        occasions: top_counts(items.iter().flat_map(|i| i.occasions.iter().map(String::as_str)), top),
        value,
        items_without_price: items.iter().filter(|i| i.price_paid.is_none()).count() as u32,
        remaining_ml_total: items.iter().filter_map(|i| i.remaining_ml).sum(),
        total_wears: items.iter().map(|i| i.wear_count).sum(),
        never_worn_count: never.len() as u32,
        never_worn: never.into_iter().take(top).map(worn).collect(),
        most_worn: most_worn.into_iter().take(top).map(worn).collect(),
        rated_items: my_scent.len() as u32,
        my_average_scent_rating: mean(my_scent),
        community_average_rating: mean(items.iter().filter_map(|i| i.community_rating).collect()),
    }
}

fn counts_text(label: &str, counts: &[Count]) -> Option<String> {
    (!counts.is_empty()).then(|| format!("{label}: {}", counts.iter().map(|c| format!("{} ({})", c.name, c.count)).collect::<Vec<_>>().join(", ")))
}

// ---- tools ---------------------------------------------------------------------------------

impl ParfumoTools {
    /// Client errors of the account tools. An unknown list is not an unknown perfume: the
    /// generic `not_found` hint (search the database) would mislead.
    pub(crate) fn ce(&self, e: parfumo_client::ClientError) -> ToolError {
        match e {
            parfumo_client::ClientError::NotFound(what) if what.starts_with("collection ") => {
                ToolError::new(ErrorCode::NotFound, format!("No {what} on this Parfumo account."))
            }
            other => self.client_err(other),
        }
    }

    /// Numeric perfume id for a write. A `Brand/Perfume` reference is resolved on the public
    /// (cached, anonymous) client: the account session is not needed to read a perfume page.
    pub(crate) async fn resolve_p_id(&self, c: &Clients, reference: &str) -> Result<u64, ToolError> {
        c.public.perfume_numeric_id(reference).await.map(|(id, _)| id).map_err(|e| self.ce(e))
    }

    pub(crate) async fn send_all(&self, c: &Clients, calls: &[WriteCall]) -> Result<Vec<String>, ToolError> {
        let mut bodies = Vec::with_capacity(calls.len());
        for call in calls {
            bodies.push(c.send(call).await.map_err(|e| self.ce(e))?);
        }
        Ok(bodies)
    }
}

#[tool_router(router = account_router, vis = "pub(crate)")]
impl ParfumoTools {
    #[tool(
        name = "parfumo_list_collections",
        description = "List the collection lists of the user's own Parfumo account with their sizes: I have, I had, Wish List, Watch List, Tested, Decants, Miniatures and custom lists, plus the custom ShelfControl columns. Use it first to learn which `collection` values the other collection tools accept. Requires a linked Parfumo account.",
        annotations(title = "List my collections", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<CollectionsOutput>()
    )]
    async fn list_collections(&self, Parameters(_p): Parameters<NoParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        self.run_account(&caller, "parfumo_list_collections", "", |c| async move {
            let shelf = c.account.shelf_default().await.map_err(|e| self.ce(e))?;
            let total: u32 = shelf.lists.iter().map(|l| l.count).sum();
            let mut lines: Vec<String> = shelf.lists.iter().map(list_line).collect();
            if !shelf.columns.is_empty() {
                lines.push(format!("Custom columns: {}", shelf.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let text = paging::digest(format!("{} lists, {} entries in total.", shelf.lists.len(), total), lines);
            Ok((text, CollectionsOutput { collections: shelf.lists, custom_columns: shelf.columns }))
        })
        .await
    }

    #[tool(
        name = "parfumo_get_collection",
        description = "Read one list of the user's own Parfumo collection with the private fields of each bottle: quantity, size, millilitres left, price paid, private note, own ratings, wear count, custom columns. Use parfumo_list_collections to find the list. Returns a paginated list of bottles; each has the perfume `id` usable with the other tools. Requires a linked Parfumo account.",
        annotations(title = "Get my collection", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<CollectionOutput>()
    )]
    async fn get_collection(&self, Parameters(p): Parameters<GetCollectionParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = p.collection.clone().unwrap_or_else(|| DEFAULT_COLLECTION.into());
        self.run_account(&caller, "parfumo_get_collection", &target, |c| async move {
            let offset = paging::offset_cursor(p.cursor.as_deref())? as usize;
            let limit = paging::clamp_limit(p.limit) as usize;
            let shelf = c.account.shelf(p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)).await.map_err(|e| self.ce(e))?;
            let total = shelf.items.len();
            if offset > 0 && offset >= total {
                return Err(ToolError::invalid("Cursor is past the end of this list. Omit `cursor` to read it from the start."));
            }
            let mut items: Vec<CollectionItem> =
                shelf.items.into_iter().skip(offset).take(limit).map(|i| if p.include_details { i } else { compact(i) }).collect();
            let keep = paging::fit_prefix(&items, MAX_STRUCTURED_CHARS - 1_000);
            items.truncate(keep);
            let end = offset + items.len();
            let next_cursor = (end < total).then(|| Cursor::Offset(end as u32).encode());
            let header = format!(
                "{}: items {}-{} of {}.{}",
                shelf.list.name,
                if items.is_empty() { 0 } else { offset + 1 },
                end,
                total,
                if next_cursor.is_some() { " More available: pass next_cursor as cursor." } else { "" }
            );
            let text = paging::digest(header, items.iter().map(item_line).collect::<Vec<_>>());
            Ok((text, CollectionOutput { collection: shelf.list, items, total_items: total as u32, next_cursor }))
        })
        .await
    }

    #[tool(
        name = "parfumo_add_to_collection",
        description = "Add a perfume to one list of the user's own Parfumo collection (default: I have). Adding a perfume that is already in the list changes nothing and returns the existing entry. Returns the entry as Parfumo holds it after the call. Collection lists can be seen by other members unless hidden. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Add to my collection", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<CollectionWriteOutput>()
    )]
    async fn add_to_collection(&self, Parameters(p): Parameters<AddToCollectionParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{} -> {} dry_run={}", p.perfume, p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION), p.dry_run);
        self.run_account(&caller, "parfumo_add_to_collection", &target, |c| async move {
            let key = perfume_key(&c.account, &p.perfume)?;
            let reference = p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION);
            let shelf = c.account.shelf(reference).await.map_err(|e| self.ce(e))?;
            if let Some(item) = find_item(&shelf, &key) {
                let out = CollectionWriteOutput {
                    dry_run: p.dry_run,
                    status: "already_present".into(),
                    item: Some(compact(item.clone())),
                    collection: Some(shelf.list.clone()),
                    ..Default::default()
                };
                return Ok((format!("Already in '{}': nothing to do.\n{}", shelf.list.name, item_line(item)), out));
            }
            let p_id = self.resolve_p_id(&c, &p.perfume).await?;
            let planned = vec![calls::collection_add(p_id, shelf.list.id)];
            if p.dry_run {
                let out = CollectionWriteOutput {
                    dry_run: true,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    collection: Some(shelf.list),
                    ..Default::default()
                };
                return Ok((write_text(&out), out));
            }
            let body = self.send_all(&c, &planned).await?.remove(0);
            if body.trim() == "0" {
                return Err(ToolError::new(
                    ErrorCode::NotFound,
                    format!("Parfumo does not know the perfume '{}'. Use parfumo_search_perfumes to find its id.", p.perfume),
                ));
            }
            let after = c.account.shelf(&shelf.list.slug).await.map_err(|e| self.ce(e))?;
            let item = after.items.iter().find(|i| i.p_id == p_id).cloned().ok_or_else(|| {
                ToolError::new(
                    ErrorCode::UpstreamError,
                    format!("Parfumo accepted the request but the perfume is not in '{}' when the list is read again. Retry later.", after.list.name),
                )
            })?;
            let out = CollectionWriteOutput {
                executed: true,
                status: "added".into(),
                planned_requests: planned,
                item: Some(compact(item)),
                collection: Some(after.list),
                ..Default::default()
            };
            Ok((write_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_move_collection_item",
        description = "Move a perfume from one list of the user's own Parfumo collection to another, or copy it with `copy: true` (it then stays in the source list). Use for example to move a Wish List perfume to I have once bought. Returns the entry in the destination list as Parfumo holds it after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Move a collection item", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        output_schema = schema_for_output::<CollectionWriteOutput>()
    )]
    async fn move_collection_item(&self, Parameters(p): Parameters<MoveCollectionItemParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{}: {} -> {} copy={} dry_run={}", p.perfume, p.from, p.to, p.copy, p.dry_run);
        self.run_account(&caller, "parfumo_move_collection_item", &target, |c| async move {
            let key = perfume_key(&c.account, &p.perfume)?;
            let source = c.account.shelf(&p.from).await.map_err(|e| self.ce(e))?;
            let dest_list = list_ref(&source.lists, &p.to)?.clone();
            if dest_list.id == source.list.id {
                return Err(ToolError::invalid("`from` and `to` are the same list."));
            }
            let item = find_item(&source, &key).ok_or_else(|| not_in_list(&p.perfume, &source.list))?.clone();
            let dest = c.account.shelf(&dest_list.slug).await.map_err(|e| self.ce(e))?;
            if let Some(existing) = dest.items.iter().find(|i| i.p_id == item.p_id) {
                let mut out = CollectionWriteOutput {
                    dry_run: p.dry_run,
                    status: "already_in_target".into(),
                    item: Some(compact(existing.clone())),
                    collection: Some(dest.list.clone()),
                    ..Default::default()
                };
                if !p.copy {
                    out.notes.push(format!(
                        "The perfume is still in '{}' too: use parfumo_remove_from_collection to take it out of that list.",
                        source.list.name
                    ));
                }
                return Ok((
                    format!(
                        "Already in '{}': nothing was sent.{}",
                        dest.list.name,
                        out.notes.first().map(|n| format!("\nNote: {n}")).unwrap_or_default()
                    ),
                    out,
                ));
            }
            let planned = vec![calls::collection_copy_move(item.entry_id, item.p_id, source.list.id, dest.list.id, p.copy)];
            if p.dry_run {
                let out = CollectionWriteOutput {
                    dry_run: true,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    item: Some(compact(item)),
                    collection: Some(dest.list),
                    ..Default::default()
                };
                return Ok((write_text(&out), out));
            }
            self.send_all(&c, &planned).await?;
            let after = c.account.shelf(&dest.list.slug).await.map_err(|e| self.ce(e))?;
            let moved = after.items.iter().find(|i| i.p_id == item.p_id).cloned().ok_or_else(|| {
                ToolError::new(
                    ErrorCode::UpstreamError,
                    format!("Parfumo accepted the request but the perfume is not in '{}' when the list is read again. Retry later.", after.list.name),
                )
            })?;
            let mut out = CollectionWriteOutput {
                executed: true,
                status: if p.copy { "copied" } else { "moved" }.into(),
                planned_requests: planned,
                item: Some(compact(moved)),
                collection: Some(after.list.clone()),
                ..Default::default()
            };
            if let Some(src) = after.lists.iter().find(|l| l.id == source.list.id) {
                out.notes.push(format!("'{}' now holds {} perfume(s) (it held {}).", src.name, src.count, source.list.count));
                if !p.copy && src.count >= source.list.count {
                    out.notes.push("The source list did not shrink: check it with parfumo_get_collection.".into());
                }
            }
            Ok((write_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_update_collection_item",
        description = "Change the private fields of a bottle in the user's own Parfumo collection: quantity, bottle size, millilitres left, price paid and currency, private note, and values of custom ShelfControl columns. Only the fields given are changed; previous values are overwritten. Parfumo has no purchase-date field (the date added is set by the site). Returns the bottle as Parfumo holds it after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Update a collection item", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<CollectionWriteOutput>()
    )]
    async fn update_collection_item(
        &self,
        Parameters(p): Parameters<UpdateCollectionItemParams>,
        caller: Caller,
    ) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{} in {} dry_run={}", p.perfume, p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION), p.dry_run);
        self.run_account(&caller, "parfumo_update_collection_item", &target, |c| async move {
            let key = perfume_key(&c.account, &p.perfume)?;
            let shelf = c.account.shelf(p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)).await.map_err(|e| self.ce(e))?;
            let item = find_item(&shelf, &key).ok_or_else(|| not_in_list(&p.perfume, &shelf.list))?.clone();
            let mut planned = Vec::new();

            if p.quantity.is_some() || p.bottle_size.is_some() || p.price_paid.is_some() || p.currency.is_some() {
                if p.quantity == Some(0) {
                    return Err(ToolError::invalid("`quantity` must be at least 1. Use parfumo_remove_from_collection to remove the bottle."));
                }
                if p.price_paid.is_some_and(|v| !v.is_finite() || v < 0.0) {
                    return Err(ToolError::invalid("`price_paid` must be a positive number."));
                }
                let currency = p.currency.as_deref().map(|c| c.trim().to_ascii_uppercase()).or(item.currency.clone()).unwrap_or_default();
                if !currency.is_empty() && !(currency.len() == 3 && currency.chars().all(|c| c.is_ascii_uppercase())) {
                    return Err(ToolError::invalid("`currency` must be a three-letter code such as EUR, USD or GBP."));
                }
                let paid = p.price_paid.or(item.price_paid);
                if paid.is_some_and(|v| v > 0.0) && currency.is_empty() {
                    return Err(ToolError::invalid("Give `currency` (e.g. \"EUR\") together with `price_paid`: this bottle has no currency yet."));
                }
                let bottle = p.bottle_size.as_deref().map(str::trim).map(str::to_string).or(item.bottle_size.clone()).unwrap_or_default();
                chars_at_most("bottle_size", &bottle, 20)?;
                planned.push(calls::collection_details(
                    item.entry_id,
                    p.quantity.or(item.quantity).unwrap_or(1),
                    &bottle,
                    &paid.map(plain_number).unwrap_or_default(),
                    &currency,
                ));
            }
            if let Some(ml) = p.remaining_ml {
                if !ml.is_finite() || !(0.0..=100_000.0).contains(&ml) {
                    return Err(ToolError::invalid("`remaining_ml` must be a positive number of millilitres."));
                }
                planned.push(calls::collection_remaining_ml(item.entry_id, &plain_number(ml)));
            }
            if let Some(note) = &p.private_note {
                chars_at_most("private_note", note, NOTE_MAX)?;
                if !(note.trim().is_empty() && item.private_note.is_none()) {
                    planned.push(calls::collection_note(item.entry_id, note));
                }
            }
            for (name, value) in p.custom.iter().flatten() {
                let column = shelf
                    .columns
                    .iter()
                    .find(|col| col.name.trim().eq_ignore_ascii_case(name.trim()) || col.name.trim().to_lowercase() == name.trim().to_lowercase())
                    .ok_or_else(|| {
                        ToolError::invalid(format!(
                            "No custom column named '{name}'. Existing custom columns: {}. Columns are created in ShelfControl on parfumo.com.",
                            if shelf.columns.is_empty() {
                                "none".to_string()
                            } else {
                                shelf.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                            }
                        ))
                    })?;
                chars_at_most("custom value", value, 500)?;
                planned.push(calls::collection_custom_value(item.entry_id, column.id, value));
            }
            if planned.is_empty() {
                return Err(ToolError::invalid(
                    "Nothing to change: give at least one of quantity, bottle_size, remaining_ml, price_paid, currency, private_note, custom.",
                ));
            }
            if p.dry_run {
                let out = CollectionWriteOutput {
                    dry_run: true,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    item: Some(compact(item)),
                    collection: Some(shelf.list),
                    ..Default::default()
                };
                return Ok((write_text(&out), out));
            }
            self.send_all(&c, &planned).await?;
            let after = c.account.shelf(&shelf.list.slug).await.map_err(|e| self.ce(e))?;
            let updated = after.items.iter().find(|i| i.entry_id == item.entry_id).cloned().ok_or_else(|| not_in_list(&p.perfume, &after.list))?;
            let out = CollectionWriteOutput {
                executed: true,
                status: "updated".into(),
                planned_requests: planned,
                item: Some(compact(updated)),
                collection: Some(after.list),
                ..Default::default()
            };
            Ok((write_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_remove_from_collection",
        description = "Remove a perfume from one list of the user's own Parfumo collection. Important: Parfumo moves a perfume removed from 'I have' into 'I had'; pass `also_from_had: true` to remove it from there too. The private fields of the removed entry (size, price, note) are lost. Removing a perfume that is not in the list is not an error. Returns the state read back from Parfumo. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Remove from my collection", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<CollectionWriteOutput>()
    )]
    async fn remove_from_collection(
        &self,
        Parameters(p): Parameters<RemoveFromCollectionParams>,
        caller: Caller,
    ) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!(
            "{} from {} also_from_had={} dry_run={}",
            p.perfume,
            p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION),
            p.also_from_had,
            p.dry_run
        );
        self.run_account(&caller, "parfumo_remove_from_collection", &target, |c| async move {
            let key = perfume_key(&c.account, &p.perfume)?;
            let shelf = c.account.shelf(p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)).await.map_err(|e| self.ce(e))?;
            let from_have = shelf.list.id == LIST_HAVE;
            let item = find_item(&shelf, &key).cloned();
            let mut planned = Vec::new();
            let mut notes = Vec::new();
            let mut p_id = item.as_ref().map(|i| i.p_id);
            if let Some(i) = &item {
                planned.push(calls::collection_remove(i.p_id, shelf.list.id));
            }
            if from_have && p.also_from_had {
                // Either the removal above puts it in "I had", or it may already be there.
                let in_had = match &item {
                    Some(_) => true,
                    None => {
                        let had = c.account.shelf("had").await.map_err(|e| self.ce(e))?;
                        let found = find_item(&had, &key).map(|i| i.p_id);
                        p_id = p_id.or(found);
                        found.is_some()
                    }
                };
                if let (true, Some(id)) = (in_had, p_id) {
                    planned.push(calls::collection_remove(id, LIST_HAD));
                }
            } else if from_have && item.is_some() {
                notes.push(
                    "Parfumo keeps a perfume removed from 'I have' in the list 'I had'. Call again with also_from_had: true to remove it entirely."
                        .to_string(),
                );
            }
            if planned.is_empty() {
                let out =
                    CollectionWriteOutput { dry_run: p.dry_run, status: "absent".into(), collection: Some(shelf.list.clone()), ..Default::default() };
                return Ok((format!("Not in '{}': nothing to do.", shelf.list.name), out));
            }
            if p.dry_run {
                let out = CollectionWriteOutput {
                    dry_run: true,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    item: item.map(compact),
                    collection: Some(shelf.list),
                    notes,
                    ..Default::default()
                };
                return Ok((write_text(&out), out));
            }
            self.send_all(&c, &planned).await?;
            let after = c.account.shelf(&shelf.list.slug).await.map_err(|e| self.ce(e))?;
            if p_id.is_some_and(|id| after.items.iter().any(|i| i.p_id == id)) {
                return Err(ToolError::new(
                    ErrorCode::UpstreamError,
                    format!("Parfumo answered, but the perfume is still in '{}' when the list is read again. Retry later.", after.list.name),
                ));
            }
            if from_have {
                let had = after.lists.iter().find(|l| l.id == LIST_HAD).map(|l| l.count).unwrap_or(0);
                notes.push(format!("'I had' now holds {had} perfume(s)."));
            }
            let out = CollectionWriteOutput {
                executed: true,
                status: "removed".into(),
                planned_requests: planned,
                collection: Some(after.list),
                notes,
                ..Default::default()
            };
            Ok((write_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_manage_custom_collection",
        description = "Create, rename, hide, unhide or delete a custom list of the user's own Parfumo collection. `delete` removes the list AND every entry in it and cannot be undone; the seven built-in lists cannot be renamed or deleted, only hidden. Returns the list as Parfumo holds it after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Manage a custom collection", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true),
        output_schema = schema_for_output::<CollectionWriteOutput>()
    )]
    async fn manage_custom_collection(
        &self,
        Parameters(p): Parameters<ManageCustomCollectionParams>,
        caller: Caller,
    ) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{:?} {} dry_run={}", p.action, p.collection.as_deref().or(p.name.as_deref()).unwrap_or(""), p.dry_run);
        self.run_account(&caller, "parfumo_manage_custom_collection", &target, |c| async move {
            let name = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
            if let Some(n) = name {
                chars_at_most("name", n, LIST_NAME_MAX)?;
            }
            if let Some(n) = &p.notes {
                chars_at_most("notes", n, LIST_NOTES_MAX)?;
            }
            let before = c.account.shelf_default().await.map_err(|e| self.ce(e))?;
            let done = |status: &str, planned: Vec<WriteCall>, list: Option<CollectionList>, executed: bool| {
                let out = CollectionWriteOutput { dry_run: p.dry_run && !planned.is_empty() && !executed, executed, status: status.into(), planned_requests: planned, collection: list, ..Default::default() };
                (write_text(&out), out)
            };
            let reread = || async { c.account.shelf_default().await.map_err(|e| self.ce(e)) };

            if p.action == ManageAction::Create {
                let name = name.ok_or_else(|| ToolError::invalid("`name` is required to create a list."))?;
                if let Some(existing) = before.lists.iter().find(|l| l.custom && l.name.trim().to_lowercase() == name.to_lowercase()) {
                    return Ok(done("already_exists", Vec::new(), Some(existing.clone()), false));
                }
                let notes = p.notes.as_deref().unwrap_or("").trim();
                if p.dry_run {
                    return Ok(done("dry_run", vec![calls::list_create(), calls::list_rename("<id of the new list>", name, notes)], None, false));
                }
                let create = calls::list_create();
                let body = c.send(&create).await.map_err(|e| self.ce(e))?;
                let id = serde_json::from_str::<serde_json::Value>(body.trim()).ok().and_then(|v| v["id"].as_u64()).ok_or_else(|| {
                    ToolError::new(ErrorCode::ParserOutdated, "Parfumo created a list but did not return its id. Check parfumo_list_collections before retrying.")
                })?;
                let rename = calls::list_rename(&id.to_string(), name, notes);
                c.send(&rename).await.map_err(|e| self.ce(e))?;
                let after = reread().await?;
                let list = after.lists.iter().find(|l| l.id == id).cloned().ok_or_else(|| ToolError::new(ErrorCode::UpstreamError, "The new list does not appear when the lists are read again. Retry later."))?;
                return Ok(done("created", vec![create, rename], Some(list), true));
            }

            let reference = p.collection.as_deref().ok_or_else(|| ToolError::invalid("`collection` (id or name of the list) is required for this action."))?;
            let Some(list) = find_list(&before.lists, reference).cloned() else {
                if p.action == ManageAction::Delete {
                    return Ok(done("absent", Vec::new(), None, false));
                }
                return Err(list_ref(&before.lists, reference).unwrap_err());
            };
            let custom_only = |what: &str| {
                if list.id <= LAST_FIXED_LIST { Err(ToolError::invalid(format!("'{}' is a built-in list: it cannot be {what}.", list.name))) } else { Ok(()) }
            };
            let (planned, status) = match p.action {
                ManageAction::Create => unreachable!("handled above"),
                ManageAction::Rename => {
                    custom_only("renamed")?;
                    if name.is_none() && p.notes.is_none() {
                        return Err(ToolError::invalid("Give `name` and/or `notes` to rename a list."));
                    }
                    let notes = p.notes.as_deref().map(str::trim).map(str::to_string).or(list.notes.clone()).unwrap_or_default();
                    (vec![calls::list_rename(&list.id.to_string(), name.unwrap_or(&list.name), &notes)], "renamed")
                }
                ManageAction::Hide | ManageAction::Unhide => {
                    let want_hidden = p.action == ManageAction::Hide;
                    if list.hidden == want_hidden {
                        return Ok(done("unchanged", Vec::new(), Some(list), false));
                    }
                    (vec![calls::list_toggle_hidden(list.id)], if want_hidden { "hidden" } else { "visible" })
                }
                ManageAction::Delete => {
                    custom_only("deleted")?;
                    (vec![calls::list_delete(list.id)], "deleted")
                }
            };
            if p.dry_run {
                let mut out = done("dry_run", planned, Some(list.clone()), false);
                if p.action == ManageAction::Delete && list.count > 0 {
                    out.1.notes.push(format!("Deleting this list also deletes its {} entries.", list.count));
                    out.0 = write_text(&out.1);
                }
                return Ok(out);
            }
            self.send_all(&c, &planned).await?;
            let after = reread().await?;
            let now = after.lists.iter().find(|l| l.id == list.id).cloned();
            let verified = match p.action {
                ManageAction::Delete => now.is_none(),
                ManageAction::Hide => now.as_ref().is_some_and(|l| l.hidden),
                ManageAction::Unhide => now.as_ref().is_some_and(|l| !l.hidden),
                _ => now.is_some(),
            };
            if !verified {
                return Err(ToolError::new(ErrorCode::UpstreamError, "Parfumo answered, but the change does not show when the lists are read again. Check with parfumo_list_collections before retrying."));
            }
            Ok(done(status, planned, now, true))
        })
        .await
    }

    #[tool(
        name = "parfumo_export_collection",
        description = "Export one whole list of the user's own Parfumo collection, with every field, as JSON (default), as a complete CSV built by this server (custom columns included), or as Parfumo's own 10-column CSV (`csv_site`). Long exports come in parts: pass `next_cursor` back until it is absent and concatenate the parts. Requires a linked Parfumo account.",
        annotations(title = "Export my collection", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<ExportOutput>()
    )]
    async fn export_collection(&self, Parameters(p): Parameters<ExportCollectionParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let format = p.format.unwrap_or_default();
        let target = format!("{} {:?}", p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION), format);
        self.run_account(&caller, "parfumo_export_collection", &target, |c| async move {
            let offset = paging::offset_cursor(p.cursor.as_deref())? as usize;
            let shelf = c.account.shelf(p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)).await.map_err(|e| self.ce(e))?;
            let total = shelf.items.len();
            let past_end = || ToolError::invalid("Cursor is past the end of this export. Omit `cursor` to start again.");
            let (format_name, items, csv, returned, rows_total) = match format {
                ExportFormat::Json => {
                    if offset > 0 && offset >= total {
                        return Err(past_end());
                    }
                    let mut items: Vec<CollectionItem> = shelf.items.into_iter().skip(offset).collect();
                    let keep = paging::fit_prefix(&items, MAX_STRUCTURED_CHARS - 1_000);
                    items.truncate(keep);
                    let n = items.len();
                    ("json", Some(items), None, n, total)
                }
                ExportFormat::Csv => {
                    if offset > 0 && offset >= total {
                        return Err(past_end());
                    }
                    let lines: Vec<String> = shelf.items.iter().map(|i| csv_row(i, &shelf.columns)).collect();
                    let (rows, n) = take_lines(&lines, offset, CSV_CHUNK_CHARS);
                    let csv = if offset == 0 { format!("{}\n{rows}", csv_header(&shelf.columns)) } else { rows };
                    ("csv", None, Some(csv), n, total)
                }
                ExportFormat::CsvSite => {
                    let file = c.account.collection_csv(shelf.list.id).await.map_err(|e| self.ce(e))?;
                    let mut lines = file.lines().map(str::to_string);
                    let header = lines.next().unwrap_or_default();
                    let lines: Vec<String> = lines.collect();
                    if offset > 0 && offset >= lines.len() {
                        return Err(past_end());
                    }
                    let (rows, n) = take_lines(&lines, offset, CSV_CHUNK_CHARS);
                    let csv = if offset == 0 { format!("{header}\n{rows}") } else { rows };
                    // A quoted field may span several lines: positions count file lines here.
                    ("csv_site", None, Some(csv), n, lines.len())
                }
            };
            let end = offset + returned;
            let next_cursor = (end < rows_total).then(|| Cursor::Offset(end as u32).encode());
            let text = format!(
                "Export of '{}' as {format_name}: rows {}-{} of {}.{}",
                shelf.list.name,
                if returned == 0 { 0 } else { offset + 1 },
                end,
                rows_total,
                if next_cursor.is_some() { " Incomplete: call again with next_cursor as cursor and append the result." } else { " Complete." }
            );
            Ok((
                text,
                ExportOutput {
                    collection: shelf.list,
                    format: format_name.into(),
                    total_items: total as u32,
                    offset: offset as u32,
                    returned: returned as u32,
                    items,
                    csv,
                    next_cursor,
                },
            ))
        })
        .await
    }

    #[tool(
        name = "parfumo_collection_stats",
        description = "Compute statistics on one list of the user's own Parfumo collection: most frequent brands, dominant notes and accords, perfumers, genders, concentrations, decades, seasons and occasions, money spent per currency, wear counts, most worn perfumes and perfumes never worn. Computed by this server from the list; use it for overviews and recommendations. Requires a linked Parfumo account.",
        annotations(title = "Collection statistics", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<StatsOutput>()
    )]
    async fn collection_stats(&self, Parameters(p): Parameters<CollectionStatsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = p.collection.clone().unwrap_or_else(|| DEFAULT_COLLECTION.into());
        self.run_account(&caller, "parfumo_collection_stats", &target, |c| async move {
            let top = p.top.unwrap_or(STATS_TOP_DEFAULT).clamp(1, STATS_TOP_MAX) as usize;
            let shelf = c.account.shelf(p.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)).await.map_err(|e| self.ce(e))?;
            let stats = compute_stats(shelf.list, &shelf.items, top);
            let mut lines = vec![format!(
                "{} perfumes, {} bottles, {} brands, {} wears logged, {} never worn.",
                stats.total_items, stats.total_bottles, stats.distinct_brands, stats.total_wears, stats.never_worn_count
            )];
            lines.extend(counts_text("Brands", &stats.brands));
            lines.extend(counts_text("Notes", &stats.notes));
            lines.extend(counts_text("Accords", &stats.accords));
            if !stats.value.is_empty() {
                lines.push(format!(
                    "Paid: {} ({} without price)",
                    stats
                        .value
                        .iter()
                        .map(|v| format!("{} {} for {} perfumes", plain_number(v.total_paid), v.currency, v.priced_items))
                        .collect::<Vec<_>>()
                        .join(", "),
                    stats.items_without_price
                ));
            }
            if !stats.most_worn.is_empty() {
                lines.push(format!(
                    "Most worn: {}",
                    stats.most_worn.iter().map(|w| format!("{} ({}×)", w.name, w.wear_count)).collect::<Vec<_>>().join(", ")
                ));
            }
            let text = paging::digest(format!("Statistics of '{}'.", stats.collection.name), lines);
            Ok((text, stats))
        })
        .await
    }

    #[tool(
        name = "parfumo_log_wear",
        description = "Log a perfume as worn (perfume of the day) on the user's own Parfumo account, now or at a past `date`, with an optional personal note. PUBLIC: the perfume worn shows on the member's public profile. Side effect: a perfume that is in none of the lists is added to 'Tested'. Not idempotent: each call logs one more wear. On this server publicly visible writes stay in dry-run mode unless the operator enabled them; the answer then shows the requests that would be made. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Log a wear", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        output_schema = schema_for_output::<WearWriteOutput>()
    )]
    async fn log_wear(&self, Parameters(p): Parameters<LogWearParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let public_writes = self.state.cfg.public_writes;
        let dry_run = p.dry_run || !public_writes;
        let target = format!("{} date={} dry_run={dry_run}", p.perfume, p.date.as_deref().unwrap_or("now"));
        self.run_account(&caller, "parfumo_log_wear", &target, |c| async move {
            let today = today_utc();
            let when = p.date.as_deref().filter(|d| !d.trim().is_empty()).map(|d| parse_wear_date(d, today)).transpose()?;
            let comment = p.comment.as_deref().map(str::trim).filter(|s| !s.is_empty());
            if let Some(text) = comment {
                chars_at_most("comment", text, WEAR_COMMENT_MAX)?;
            }
            let past_day = when.as_deref().is_some_and(|w| w[..10] != ymd_text(today));
            let p_id = self.resolve_p_id(&c, &p.perfume).await?;
            let plan = |wear: &str| {
                let mut calls_ = vec![calls::wear_log(p_id)];
                if let Some(w) = &when {
                    calls_.push(calls::wear_set_date(wear, w));
                }
                if let Some(text) = comment {
                    calls_.push(calls::wear_set_comment(wear, text));
                }
                if past_day {
                    calls_.push(calls::wear_clear_current());
                }
                calls_
            };
            let mut notes = vec![
                "A logged wear is public: the perfume shows as currently worn on the member's Parfumo profile.".to_string(),
                "If the perfume is in none of the lists, Parfumo adds it to 'Tested'.".to_string(),
            ];
            if dry_run {
                if !public_writes {
                    notes.insert(0, crate::public_gate::GATE_NOTE.to_string());
                }
                let out = WearWriteOutput { dry_run: true, status: "dry_run".into(), public_writes_enabled: public_writes, planned_requests: plan("<id of the new wear>"), notes, ..Default::default() };
                return Ok((wear_write_text(&out), out));
            }

            let before: Vec<u64> = c.account.wears_overview().await.map_err(|e| self.ce(e))?.iter().map(|w| w.wear_id).collect();
            let log = calls::wear_log(p_id);
            self.send_public(&c, "parfumo_log_wear", std::slice::from_ref(&log)).await?;
            let mut sent = vec![log];
            let after = c.account.wears_overview().await.map_err(|e| self.ce(e))?;
            let Some(new) = after.iter().find(|w| !before.contains(&w.wear_id)).cloned() else {
                notes.push("Parfumo accepted the wear, but it was not found in the tracker afterwards: date and note were not applied. Check with parfumo_list_wears.".into());
                let out = WearWriteOutput { executed: true, status: "logged_unverified".into(), public_writes_enabled: true, planned_requests: sent, notes, ..Default::default() };
                return Ok((wear_write_text(&out), out));
            };
            let follow_up: Vec<WriteCall> = plan(&new.wear_id.to_string()).into_iter().skip(1).collect();
            self.send_all(&c, &follow_up).await?;
            let wear = if follow_up.is_empty() {
                new
            } else {
                let again = c.account.wears_overview().await.map_err(|e| self.ce(e))?;
                again.into_iter().find(|w| w.wear_id == new.wear_id).unwrap_or(new)
            };
            sent.extend(follow_up);
            let out = WearWriteOutput { executed: true, status: "logged".into(), public_writes_enabled: true, planned_requests: sent, wear: Some(wear), notes, ..Default::default() };
            Ok((wear_write_text(&out), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_list_wears",
        description = "List the wears (perfume of the day) logged on the user's own Parfumo account over a period, most recent first. Default period: the current month and the two before it; a call covers 12 calendar months at most (one request per month). Returns each wear with its `wear_id`, perfume, date and personal note. Requires a linked Parfumo account.",
        annotations(title = "List my wears", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<WearsOutput>()
    )]
    async fn list_wears(&self, Parameters(p): Parameters<ListWearsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{}..{}", p.from.as_deref().unwrap_or(""), p.to.as_deref().unwrap_or(""));
        self.run_account(&caller, "parfumo_list_wears", &target, |c| async move {
            let offset = paging::offset_cursor(p.cursor.as_deref())? as usize;
            let limit = paging::clamp_limit(p.limit) as usize;
            let to = match p.to.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(s) => parse_ymd("to", s)?,
                None => today_utc(),
            };
            let from = match p.from.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(s) => parse_ymd("from", s)?,
                None => {
                    let months = to.0 * 12 + i64::from(to.1) - 1 - 2;
                    (months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1)
                }
            };
            if from > to {
                return Err(ToolError::invalid("`from` is after `to`."));
            }
            let months = months_between(from, to);
            if months.len() as u32 > MAX_WEAR_MONTHS {
                return Err(ToolError::invalid(format!(
                    "The period spans {} calendar months; a call reads {MAX_WEAR_MONTHS} at most. Narrow `from`/`to` and call again for the rest.",
                    months.len()
                )));
            }
            let (from_s, to_s) = (ymd_text(from), ymd_text(to));
            let mut wears: Vec<Wear> = Vec::new();
            for (y, m) in months {
                let month = c.account.wears_of_month(y, m).await.map_err(|e| self.ce(e))?;
                wears.extend(
                    month.into_iter().filter(|w| w.worn_at.as_deref().is_none_or(|at| &at[..10] >= from_s.as_str() && &at[..10] <= to_s.as_str())),
                );
            }
            wears.sort_by(|a, b| b.worn_at.cmp(&a.worn_at).then(b.wear_id.cmp(&a.wear_id)));
            wears.dedup_by_key(|w| w.wear_id);
            let total = wears.len();
            if offset > 0 && offset >= total {
                return Err(ToolError::invalid("Cursor is past the end of this period. Omit `cursor` to read it from the start."));
            }
            let mut page: Vec<Wear> = wears.into_iter().skip(offset).take(limit).collect();
            let keep = paging::fit_prefix(&page, MAX_STRUCTURED_CHARS - 1_000);
            page.truncate(keep);
            let end = offset + page.len();
            let next_cursor = (end < total).then(|| Cursor::Offset(end as u32).encode());
            let header = format!(
                "{total} wear(s) from {from_s} to {to_s}; {} returned.{}",
                page.len(),
                if next_cursor.is_some() { " More available: pass next_cursor as cursor." } else { "" }
            );
            let text = paging::digest(header, page.iter().map(wear_line).collect::<Vec<_>>());
            Ok((text, WearsOutput { from: from_s, to: to_s, wears: page, total_in_period: total as u32, next_cursor }))
        })
        .await
    }

    #[tool(
        name = "parfumo_delete_wear",
        description = "Delete one logged wear from the user's own Parfumo account, by the `wear_id` returned by parfumo_list_wears. Cannot be undone. Deleting a wear that no longer exists is not an error. Returns whether the wear is still listed in the tracker after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Delete a wear", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<WearWriteOutput>()
    )]
    async fn delete_wear(&self, Parameters(p): Parameters<DeleteWearParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{} dry_run={}", p.wear_id, p.dry_run);
        self.run_account(&caller, "parfumo_delete_wear", &target, |c| async move {
            if p.wear_id == 0 {
                return Err(ToolError::invalid("`wear_id` must be the identifier returned by parfumo_list_wears."));
            }
            let public_writes_enabled = self.state.cfg.public_writes;
            let planned = vec![calls::wear_delete(p.wear_id)];
            let listed = c.account.wears_overview().await.map_err(|e| self.ce(e))?.into_iter().find(|w| w.wear_id == p.wear_id);
            if p.dry_run {
                let mut out = WearWriteOutput { dry_run: true, status: "dry_run".into(), public_writes_enabled, planned_requests: planned, wear: listed, ..Default::default() };
                if out.wear.is_none() {
                    out.notes.push("This wear is not on the tracker overview page: it may be older, or already deleted.".into());
                }
                return Ok((wear_write_text(&out), out));
            }
            // Sent even when the overview does not show the wear: the overview may not hold
            // the whole history, and deleting an absent wear changes nothing.
            self.send_all(&c, &planned).await?;
            let still = c.account.wears_overview().await.map_err(|e| self.ce(e))?.into_iter().any(|w| w.wear_id == p.wear_id);
            let status = match (listed.is_some(), still) {
                (_, true) => "still_listed",
                (true, false) => "deleted",
                (false, false) => "absent",
            };
            let mut out = WearWriteOutput { executed: true, status: status.into(), public_writes_enabled, planned_requests: planned, wear: listed, ..Default::default() };
            match status {
                "still_listed" => out.notes.push("Parfumo answered, but the wear is still on the tracker page. Check with parfumo_list_wears before retrying.".into()),
                "absent" => out.notes.push("The wear was not on the tracker overview before the call; the deletion request was sent anyway. Check with parfumo_list_wears if in doubt.".into()),
                _ => {}
            }
            Ok((wear_write_text(&out), out))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, brand: &str, paid: Option<(f64, &str)>, worn: u32) -> CollectionItem {
        CollectionItem {
            entry_id: 1,
            perfume_id: format!("{brand}/{name}"),
            p_id: 1,
            name: name.into(),
            brand: Some(brand.into()),
            url: format!("https://www.parfumo.com/Perfumes/{brand}/{name}"),
            year: Some(2015),
            price_paid: paid.map(|p| p.0),
            currency: paid.map(|p| p.1.to_string()),
            wear_count: worn,
            notes: vec!["Bergamot".into(), "Vetiver".into()],
            quantity: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn stats_rank_brands_notes_value_and_wears() {
        let items = vec![
            item("A", "Dior", Some((85.0, "EUR")), 3),
            item("B", "Dior", Some((100.5, "EUR")), 0),
            item("C", "Chanel", Some((90.0, "USD")), 7),
            item("D", "Hermes", None, 0),
        ];
        let s = compute_stats(CollectionList::default(), &items, 2);
        assert_eq!((s.total_items, s.total_bottles, s.distinct_brands), (4, 4, 3));
        assert_eq!(s.brands, vec![Count { name: "Dior".into(), count: 2 }, Count { name: "Chanel".into(), count: 1 }]);
        assert_eq!(s.notes[0], Count { name: "Bergamot".into(), count: 4 });
        assert_eq!(s.decades, vec![Count { name: "2010s".into(), count: 4 }]);
        assert_eq!(s.value[0], CurrencyValue { currency: "EUR".into(), total_paid: 185.5, priced_items: 2, average_paid: 92.75 });
        assert_eq!(s.value[1].currency, "USD", "currencies are never added together");
        assert_eq!((s.items_without_price, s.total_wears, s.never_worn_count), (1, 10, 2));
        assert_eq!(s.most_worn.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["C", "A"]);
        assert_eq!(s.never_worn.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["B", "D"]);
        assert_eq!(s.my_average_scent_rating, None);
        let empty = compute_stats(CollectionList::default(), &[], 5);
        assert_eq!((empty.total_items, empty.total_wears), (0, 0));
        assert!(empty.brands.is_empty() && empty.value.is_empty());
    }

    #[test]
    fn csv_quotes_every_field_and_keeps_custom_columns() {
        let cols = vec![CustomColumn { id: 178, name: "Gift".into() }];
        let mut i = item("A \"special\"", "Dior", Some((85.0, "EUR")), 0);
        i.private_note = Some("line one\nline, two".into());
        i.custom.insert("Gift".into(), "1".into());
        let header = csv_header(&cols);
        let row = csv_row(&i, &cols);
        assert!(header.starts_with("\"Perfume\",\"Brand\"") && header.contains("\"Gift\",\"Perfume Id\""));
        assert!(row.starts_with("\"A \"\"special\"\"\",\"Dior\",\"2015\""), "{row}");
        assert!(row.contains("\"line one\nline, two\""));
        assert!(row.contains("\"85\",\"EUR\""));
        // Same number of fields in the header and in a row (quotes balanced, commas outside).
        let fields = |s: &str| s.split("\",\"").count();
        assert_eq!(fields(&header), fields(&row));
    }

    #[test]
    fn csv_chunks_cover_every_row_once() {
        let lines: Vec<String> = (0..50).map(|i| format!("\"row {i}\",\"{}\"", "x".repeat(900))).collect();
        let (mut offset, mut seen) = (0, 0);
        loop {
            let (chunk, n) = take_lines(&lines, offset, CSV_CHUNK_CHARS);
            assert!(n >= 1 && chunk.chars().count() <= CSV_CHUNK_CHARS + 1_000);
            seen += n;
            offset += n;
            if offset >= lines.len() {
                break;
            }
        }
        assert_eq!(seen, 50);
        assert_eq!(take_lines(&["y".repeat(50_000)], 0, CSV_CHUNK_CHARS).1, 1, "an oversized row is still returned");
    }

    #[test]
    fn dates_are_validated() {
        let today = (2026, 11, 12);
        assert_eq!(parse_wear_date("2026-11-10", today).unwrap(), "2026-11-10T12:00");
        assert_eq!(parse_wear_date("2026-11-12T07:05", today).unwrap(), "2026-11-12T07:05");
        assert_eq!(parse_wear_date("2026-11-12 23:59:30", today).unwrap(), "2026-11-12T23:59");
        for bad in ["2026-11-13", "2026-02-30", "12/11/2026", "2026-11-12T25:00", "1999-01-01", "yesterday"] {
            assert!(parse_wear_date(bad, today).is_err(), "{bad}");
        }
        assert_eq!(parse_ymd("to", "2024-02-29").unwrap(), (2024, 2, 29));
        assert_eq!(months_between((2025, 11, 20), (2026, 2, 3)), vec![(2026, 2), (2026, 1), (2025, 12), (2025, 11)]);
        assert_eq!(months_between((2026, 3, 1), (2026, 3, 31)), vec![(2026, 3)]);
        assert_eq!(plain_number(85.0), "85");
        assert_eq!(plain_number(12.5), "12.5");
    }
}
