//! Similar perfumes through the application interface (`docs/APP_API.md`).
//!
//! `GET /similar` lists what members voted similar to a perfume (`type=similar`) and what
//! they suggest layering with it (`type=layer`): the two values the application sends.
//! Its rows carry no address: the `Brand/Perfume` id of a row is known at once when the
//! perfume was met in a listing, and is read from its record otherwise. The interface has
//! nothing about dupes: the dupes directory stays the public page of the web site.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient, RECORD_TTL,
    app_db::{count, number, text},
    models::{DupeGroup, SimilarOptions, SimilarPerfume, SimilarPerfumes},
    parse,
    site::routes,
};

/// Rows asked per list: what the application asks, and as many as the web page shows.
const LIST_LIMIT: u32 = 30;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

impl ParfumoClient {
    /// The `Brand/Perfume` id of a perfume met before, without any call.
    fn known_perfume(&self, p_id: u64) -> Option<String> {
        self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).values().find(|(id, _)| *id == p_id).map(|(_, slug)| slug.clone())
    }

    fn link(&self, item: &mut SimilarPerfume, id: String) {
        item.url = Some(self.t.absolute(&routes::perfume(&id)));
        item.id = Some(id);
    }

    /// A row of `GET /similar`. The consensus and its votes come with every row.
    fn similar_row(&self, row: &Value) -> Option<SimilarPerfume> {
        let mut item = SimilarPerfume {
            p_id: count(row, "id")?,
            // `p_name` repeats the concentration in brackets.
            name: text(row, "con_ext_name").or_else(|| text(row, "p_name"))?,
            brand: text(row, "b_name"),
            concentration: text(row, "con_ext_concentration"),
            percent: number(row, "percentage").map(|p| p as f32),
            votes: count(row, "cnt").and_then(|n| u32::try_from(n).ok()),
            image_url: text(row, "url_image"),
            ..SimilarPerfume::default()
        };
        if let Some(id) = self.known_perfume(item.p_id) {
            self.link(&mut item, id);
        }
        Some(item)
    }

    /// One list of `GET /similar`: `kind` is `similar` or `layer`.
    async fn app_similar_list(&self, kind: &str, p_id: u64) -> Result<Vec<SimilarPerfume>, ClientError> {
        let params = [pair("action", "list"), pair("type", kind), pair("p_id", p_id), pair("limit", LIST_LIMIT)];
        let answer = self.app_get("/similar", &params).await?;
        let rows = answer
            .get("perfumes")
            .and_then(Value::as_array)
            .filter(|_| text(&answer, "status").as_deref() == Some("success"))
            .ok_or_else(|| ClientError::ParserOutdated(format!("/app/v1/similar: no list for type={kind}")))?;
        Ok(rows.iter().filter_map(|row| self.similar_row(row)).collect())
    }

    /// The groups of the dupes directory a perfume appears in. The application interface
    /// does not know the directory: this is the public page of the web site, read as before.
    async fn web_dupes(&self, name: &str, id: &str) -> Result<Vec<DupeGroup>, ClientError> {
        let query = url::form_urlencoded::Serializer::new(String::new()).append_pair("q", name).finish();
        let (_, body) = self.t.get(&format!("{}?{query}", routes::GET_DUPES), Some(RECORD_TTL), false, true).await?;
        let groups = parse::parse_dupes(&body)?;
        Ok(groups.into_iter().filter(|g| g.original.id == id || g.dupes.iter().any(|d| d.perfume.id == id)).collect())
    }

    /// Similar perfumes and layering suggestions through the application interface, the
    /// dupes from the web site. `None` when the reference is not found through the
    /// interface: the caller then asks the web site, which knows every address.
    pub(crate) async fn app_similar_perfumes(&self, r: &str, opts: SimilarOptions) -> Result<Option<SimilarPerfumes>, ClientError> {
        let p_id = match self.app_perfume_id(r).await {
            Ok(p_id) => p_id,
            Err(ClientError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        // The record gives the name the dupes directory is searched by, and the id when it
        // is not remembered.
        let remembered = if opts.include_dupes { None } else { self.known_perfume(p_id) };
        let (perfume_id, name) = match remembered {
            Some(id) => (id, None),
            None => {
                let record = self.app_get_perfume(&p_id.to_string()).await?;
                (record.id, opts.include_dupes.then_some(record.name))
            }
        };
        let mut similar = self.app_similar_list("similar", p_id).await?;
        let mut warnings = Vec::new();

        // The first rows not met before get their id from their record, one call each.
        for item in similar.iter_mut().take(opts.resolve_urls).filter(|item| item.id.is_none()) {
            match self.app_get_perfume(&item.p_id.to_string()).await {
                Ok(record) => self.link(item, record.id),
                Err(e) => {
                    warnings.push(format!("links of similar perfumes not resolved: {e}"));
                    break;
                }
            }
        }

        let mut layering = Vec::new();
        if opts.include_layering {
            // What the interface says of a row: its links are ours, and differ between the lists.
            let said = |list: &[SimilarPerfume]| list.iter().map(|p| (p.p_id, p.percent, p.votes)).collect::<Vec<_>>();
            match self.app_similar_list("layer", p_id).await {
                // The same rows with the same votes would mean that the type was ignored:
                // similar perfumes are not shown as layering suggestions.
                Ok(list) if !list.is_empty() && said(&list) == said(&similar) => {
                    warnings.push("layering suggestions unavailable: the application interface answered the list of similar perfumes".into());
                }
                Ok(list) => layering = list,
                Err(e) => warnings.push(format!("layering suggestions unavailable: {e}")),
            }
        }

        let mut dupes = Vec::new();
        if let Some(name) = &name {
            match self.web_dupes(name, &perfume_id).await {
                Ok(groups) => dupes = groups,
                Err(e) => warnings.push(format!("dupes directory unavailable: {e}")),
            }
        }

        Ok(Some(SimilarPerfumes { perfume_id, similar, layering, dupes, warnings }))
    }
}
