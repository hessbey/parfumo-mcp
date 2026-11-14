//! The wear log through the application interface (`docs/APP_API.md`).
//!
//! `GET /tracker_calendar` lists the wears of one month and `GET /tracker_overview` the
//! history, most recent first, by windows. Both give the same rows.
//!
//! The identifier of a wear is `l_id`. The web pages give it that very name
//! (`#tablerow{l_id}`, `data-l_id`) and their write calls take it as `id`; the numbers
//! recorded on both sides fit one sequence (22205264 on the web page, 22231107 here the
//! day after). It is therefore handed on as the `wear_id` the tools date, annotate and
//! delete a wear by. One and the same wear has not been read on both sides.
//!
//! The name is `p_name`. Where other answers pair it with a `con_ext_name`, it is the name
//! as the web page writes it, concentration in brackets (`Sauvage (Eau de Toilette)`), and
//! `con_ext_name` the name without it; where they pair it with a `concentration`, it is
//! the bare name. A wear has all three, and the one recorded is of a perfume without a
//! concentration, where both names are equal: a row that gives a concentration its
//! `p_name` does not carry is therefore not read.
//!
//! Dates keep the convention of the web parser: the local time the site shows the member.
//! It is read from `wear_date_readable`, the text of the web page. `wear_date` is the
//! instant, in UTC: it is never handed on as a local time, and only serves to check the text.
//!
//! An answer that is not surely the wears asked for is not read: the caller then asks the
//! web site, as it does without a session of the interface.

use std::collections::HashSet;

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{count, number, text},
    models::Wear,
    parse,
    site::routes,
};

/// Rows asked per window of the history: what the application asks.
const WINDOW: usize = 20;
/// Windows read at most: the hundred most recent wears. The tools only look there for a
/// wear just logged or about to be deleted, and say so when they do not find it.
const MAX_WINDOWS: usize = 5;
/// Time zones run from UTC−12 to UTC+14: a local date is never further from the instant.
const ZONE_SPAN_SECS: i64 = 14 * 3600;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The interface answers a member it does not recognise with `login_status: 0` and no row.
fn recognised(answer: &Value) -> bool {
    count(answer, "login_status") == Some(1)
}

/// The local date and time of a wear (`YYYY-MM-DDTHH:MM`), from the text the site shows.
/// A text in another form, or one that names another day than the instant allows (a month
/// read as a day), gives no date rather than a wrong one.
fn worn_at(row: &Value) -> Option<String> {
    let local = parse::parse_wear_when(&text(row, "wear_date_readable")?)?;
    let Some(instant) = number(row, "wear_date").map(|ts| ts as i64) else { return Some(local) };
    let day = &local[..10];
    (parse::iso_date(instant - ZONE_SPAN_SECS).as_str() <= day && day <= parse::iso_date(instant + ZONE_SPAN_SECS).as_str()).then_some(local)
}

/// The member's note on a wear, on one line as the web page shows it. `Err` for a shape
/// never seen (the answers recorded only have `null`).
fn comment(row: &Value) -> Result<Option<String>, ()> {
    match row.get("comment") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(note)) => Ok(Some(note.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|n| !n.is_empty())),
        Some(_) => Err(()),
    }
}

impl ParfumoClient {
    /// A row of the calendar or of the history. `None` when it lacks what designates the
    /// wear or its perfume, or what names it as the web page does: such a row is never
    /// dropped, the whole answer is left unread.
    fn wear_row(&self, row: &Value) -> Option<Wear> {
        let wear_id = count(row, "l_id")?;
        let id = format!("{}/{}", text(row, "b_name_url")?, text(row, "p_name_url")?);
        let name = text(row, "p_name")?;
        // The web page names a perfume with its concentration in brackets. Whether
        // `p_name` does so here was not recorded: a bare name is not handed on in its place.
        if text(row, "concentration").is_some_and(|concentration| !name.contains(&format!("({concentration})"))) {
            return None;
        }
        let comment = comment(row).ok()?;
        if let Some(p_id) = count(row, "p_id") {
            self.remember_perfume(&id, p_id);
        }
        Some(Wear {
            wear_id,
            name,
            brand: text(row, "b_name"),
            url: self.t.absolute(&routes::perfume(&id)),
            perfume_id: id,
            worn_at: worn_at(row),
            comment,
        })
    }

    /// The rows of an answer, in its order. `None` when the answer has no list or a row
    /// cannot be read.
    fn wear_rows(&self, answer: &Value) -> Option<Vec<Wear>> {
        answer.get("items")?.as_array()?.iter().map(|row| self.wear_row(row)).collect()
    }

    /// Wears of one calendar month (`GET /tracker_calendar`). `None` when the answer is
    /// not the month asked, or does not hold as many wears as it counts.
    pub(crate) async fn app_wears_of_month(&self, year: i64, month: u32) -> Result<Option<Vec<Wear>>, ClientError> {
        let answer = self.app_get("/tracker_calendar", &[pair("y", year), pair("m", month)]).await?;
        // Without a month it understands, the interface answers the current one.
        let asked = number(&answer, "year") == Some(year as f64) && number(&answer, "month") == Some(f64::from(month));
        if !recognised(&answer) || !asked {
            return Ok(None);
        }
        let Some(wears) = self.wear_rows(&answer) else { return Ok(None) };
        let counted = answer.get("totals").and_then(|totals| number(totals, "wears"));
        Ok(Some(wears).filter(|wears| counted == Some(wears.len() as f64)))
    }

    /// The most recent wears, in the order of the interface (`GET /tracker_overview`):
    /// its windows are read one after the other, as the application does, up to
    /// [`MAX_WINDOWS`]. `None` when a window is not the one asked or cannot be read.
    pub(crate) async fn app_wears_overview(&self) -> Result<Option<Vec<Wear>>, ClientError> {
        let (mut wears, mut seen, mut read) = (Vec::new(), HashSet::new(), 0);
        for _ in 0..MAX_WINDOWS {
            let answer = self.app_get("/tracker_overview", &[pair("offset", read), pair("limit", WINDOW)]).await?;
            if !recognised(&answer) || number(&answer, "offset") != Some(read as f64) {
                return Ok(None);
            }
            // `has_more` was only seen as a boolean. Another shape is not guessed: a wrong
            // guess would pass a cut history for a whole one.
            let (Some(rows), Some(more)) = (self.wear_rows(&answer), answer.get("has_more").and_then(Value::as_bool)) else { return Ok(None) };
            read += rows.len();
            let known = wears.len();
            // A wear logged between two calls shifts the windows: a row may come twice.
            wears.extend(rows.into_iter().filter(|w| seen.insert(w.wear_id)));
            if !more || wears.len() == known {
                break;
            }
        }
        Ok(Some(wears))
    }
}
