//! The page of a fragrance note through the application interface (`docs/APP_API.md`).
//!
//! `GET /fragrance_notes` knows notes and categories by number and gives almost no web
//! address: none for a note, one for a category on its own page and in the overview of
//! the top-level categories, and its rows of perfumes lack the slug of their brand. A
//! name is not an address either: the note "Bergamot blossom" lives at
//! `Bergamotte_blossom`, and `Candied_bergamot_peel` is the address of another note than
//! the one named "Candied bergamot peel".
//!
//! So only what the interface designates with certainty is read here: a note by its
//! number or by a name a single note bears, a top-level category by its address. The rest
//! is left to the web site, and an address the interface does not give stays empty.

use std::{collections::HashMap, sync::MutexGuard};

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{count, gender, number, out_of_ten, text},
    models::{EntityRef, Listing, NotePage, PerfumeSummary},
    site::routes,
};

/// The one call behind every read of this module; `feature` chooses the read.
const CALL: &str = "/fragrance_notes";
/// Rows asked per page: what the application asks, and what the web page shows.
const PER_PAGE: u32 = 20;
/// Names remembered at most: they are whatever the caller typed.
const MAX_NAMES: usize = 1_000;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// What the calls of this module already told, for the life of the client: a listing is
/// read page after page under the same reference, which is looked up once.
#[derive(Default)]
pub(crate) struct NoteMemory {
    /// The number of the note a lower-case name designates; `None` when the search
    /// designates none.
    names: HashMap<String, Option<u64>>,
    /// Number and slug of the top-level categories, once the overview was read.
    categories: Option<Vec<(u64, String)>>,
}

impl ParfumoClient {
    fn note_memory(&self) -> MutexGuard<'_, NoteMemory> {
        self.t.notes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The number of the note a name designates (`feature=search`): the one note of the
    /// answer that bears it, whatever the letter case. `None` when none does, or several:
    /// the site has notes of the same name, and nothing here tells them apart. One call
    /// the first time a name is met, none after.
    async fn app_note_id(&self, name: &str) -> Result<Option<u64>, ClientError> {
        let wanted = name.to_lowercase();
        let remembered = self.note_memory().names.get(&wanted).copied();
        if let Some(nr) = remembered {
            return Ok(nr);
        }
        let found = self.app_get(CALL, &[pair("feature", "search"), pair("q", name)]).await?;
        let bearing: Vec<&Value> = found
            .get("notes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|note| text(note, "name").is_some_and(|n| n.to_lowercase() == wanted))
            .collect();
        let nr = match bearing.as_slice() {
            [only] => count(only, "id"),
            _ => None,
        };
        let mut memory = self.note_memory();
        if memory.names.len() >= MAX_NAMES {
            memory.names.clear();
        }
        memory.names.insert(wanted, nr);
        Ok(nr)
    }

    /// Number and slug of the top-level categories (`feature=overview`): the only
    /// categories the interface gives the address of before their page is read. One call
    /// the first time, none after.
    async fn app_note_categories(&self) -> Result<Vec<(u64, String)>, ClientError> {
        let remembered = self.note_memory().categories.clone();
        if let Some(known) = remembered {
            return Ok(known);
        }
        let overview = self.app_get(CALL, &[pair("feature", "overview")]).await?;
        let categories = overview.get("categories").and_then(Value::as_array);
        let known: Vec<(u64, String)> = categories.into_iter().flatten().filter_map(|c| Some((count(c, "id")?, text(c, "name_url")?))).collect();
        // An answer without categories is not remembered: it is not what the overview is.
        if !known.is_empty() {
            self.note_memory().categories = Some(known.clone());
        }
        Ok(known)
    }

    /// A category of notes; it has an address only when its slug is known. Its number is
    /// not handed on: `numeric_id` is the number of a note, which `get_note` takes as a
    /// reference, and categories are numbered apart (the note 311 and the category 37 are
    /// both "Bergamot").
    fn app_note_category(&self, slug: Option<String>, name: String) -> EntityRef {
        let url = slug.as_ref().map(|slug| self.t.absolute(&routes::note(slug, true))).unwrap_or_default();
        EntityRef { id: slug.unwrap_or_default(), numeric_id: None, name, url }
    }

    /// A row of perfumes of a note or of a category. Not a document of the search index:
    /// fewer fields, other names, and no slug of the brand. The `Brand/Perfume` id and the
    /// address are therefore only known for a perfume met before, in a listing or by its record.
    fn app_note_row(&self, row: &Value) -> Option<PerfumeSummary> {
        let p_id = count(row, "id")?;
        let id = self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).values().find(|(met, _)| *met == p_id).map(|(_, id)| id.clone());
        let votes = count(row, "total_votings");
        Some(PerfumeSummary {
            url: id.as_ref().map(|id| self.t.absolute(&routes::perfume(id))).unwrap_or_default(),
            id: id.unwrap_or_default(),
            p_id: Some(p_id),
            name: text(row, "p_name")?,
            brand: text(row, "b_name"),
            year: count(row, "release_year").and_then(|y| u16::try_from(y).ok()),
            concentration: text(row, "concentration"),
            rating: votes.and_then(|_| number(row, "avg_voting")).filter(|r| *r > 0.0).map(out_of_ten),
            rating_count: votes.and_then(|n| u32::try_from(n).ok()),
            gender: gender(row, "gender"),
            image_url: text(row, "url_image_240"),
            ..PerfumeSummary::default()
        })
    }

    /// The page of a note (`feature=note`) or of a category of notes (`feature=category`),
    /// when the interface designates it with certainty. `None` otherwise: the caller then
    /// reads the web site, which knows every address.
    pub(crate) async fn app_note(&self, r: &str, page: u32) -> Result<Option<NotePage>, ClientError> {
        match self.app_note_page(r, page).await {
            // A number, or a call, the interface does not know: the web site says what it is.
            Err(ClientError::NotFound(_)) => Ok(None),
            other => other,
        }
    }

    async fn app_note_page(&self, r: &str, page: u32) -> Result<Option<NotePage>, ClientError> {
        let (absolute, path) = self.ref_path(r)?;
        // What is asked: a category by its address, or a note by its number or its name.
        let (asked_slug, nr, known) = if absolute {
            // The address of a note cannot be checked, the interface gives none; that of a
            // category can, against the overview and then against its own page.
            let Some(slug) = path.strip_prefix(routes::NOTE_CATEGORY_PREFIX).and_then(|rest| rest.split('/').find(|p| !p.is_empty())) else {
                return Ok(None);
            };
            let known = self.app_note_categories().await?;
            let Some(nr) = known.iter().find(|(_, known)| known.eq_ignore_ascii_case(slug)).map(|(nr, _)| *nr) else { return Ok(None) };
            (Some(slug), nr, known)
        } else {
            let name = path.trim();
            let nr = if name.chars().all(|c| c.is_ascii_digit()) {
                name.parse::<u64>().ok().filter(|nr| *nr > 0)
            } else if name.contains(['_', '/']) {
                // Written as a slug: the words of a slug may be the name of another note.
                None
            } else {
                self.app_note_id(name).await?
            };
            let Some(nr) = nr else { return Ok(None) };
            (None, nr, Vec::new())
        };

        let (feature, key) = if asked_slug.is_some() { ("category", "cat_id") } else { ("note", "note_id") };
        let page = page.max(1);
        let params = [pair("feature", feature), pair(key, nr), pair("page", page), pair("per_page", PER_PAGE)];
        let answer = self.app_get(CALL, &params).await?;
        // No head, or the head of another number than asked: a number the interface does
        // not know. What it answers for one was not recorded, and a wrong note must not pass.
        let Some(head) = answer.get(feature).filter(|head| count(head, "id") == Some(nr)) else { return Ok(None) };
        // The answer says which page it is: the rows of another page are not those asked.
        if number(&answer, "page").is_some_and(|answered| answered != f64::from(page)) {
            return Ok(None);
        }
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/fragrance_notes: {what}"));
        let name = text(head, "name").ok_or_else(|| outdated("no name"))?;
        let (note, related) = match asked_slug {
            // The page of a category repeats its slug: it must be the one that was asked.
            Some(asked) => match text(head, "name_url").filter(|slug| slug.eq_ignore_ascii_case(asked)) {
                Some(slug) => (self.app_note_category(Some(slug), name), answer.get("related_categories")),
                None => return Ok(None),
            },
            // The interface gives no address of a note: none is made up.
            None => (EntityRef { id: String::new(), numeric_id: Some(nr), name, url: String::new() }, head.get("categories")),
        };
        let slug_of = |c: &Value| known.iter().find(|(nr, _)| Some(*nr) == count(c, "id")).map(|(_, slug)| slug.clone());
        let categories = related
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| Some(self.app_note_category(slug_of(c), text(c, "name")?)))
            .collect();
        let rows = answer.get("perfumes").and_then(Value::as_array).ok_or_else(|| outdated("no perfumes"))?;
        let total = |key: &str| number(&answer, key).filter(|n| *n >= 0.0).map(|n| n as u32);
        Ok(Some(NotePage {
            note,
            is_category: asked_slug.is_some(),
            categories,
            perfumes: Listing {
                items: rows.iter().filter_map(|row| self.app_note_row(row)).collect(),
                page,
                // An empty page is the last one, whatever the count says: the web site stops
                // at 1,000 rows, and whether the interface goes further was not recorded.
                total_pages: total("total_pages").map(|pages| if rows.is_empty() { pages.min(page) } else { pages }),
                total_items: total("total_results"),
            },
        }))
    }
}
