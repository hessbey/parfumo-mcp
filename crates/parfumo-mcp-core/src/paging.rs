//! Pagination (`limit` / `cursor`) and the response size cap shared by all tools.

use std::future::Future;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use parfumo_client::{ClientError, Listing};
use serde::Serialize;

use crate::errors::ToolError;

pub const DEFAULT_LIMIT: u32 = 25;
pub const MAX_LIMIT: u32 = 100;
/// Budget for the structured payload; with the short text digest a response stays around
/// 25 000 characters.
pub const MAX_STRUCTURED_CHARS: usize = 22_000;
/// Budget for the human-readable digest.
pub const MAX_DIGEST_CHARS: usize = 3_000;
/// Upper bound of Parfumo pages fetched by one tool call (one request per second each).
pub const MAX_SITE_PAGES_PER_CALL: u32 = 5;

/// Position in a site listing: native page number (1-based) and items to skip on it.
pub type Pos = (u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Page(Pos),
    Offset(u32),
    /// Messages older than this one: `(message id, unix time)`.
    Before(u64, i64),
}

impl Cursor {
    pub fn encode(self) -> String {
        let raw = match self {
            Cursor::Page((page, skip)) => format!("p:{page}:{skip}"),
            Cursor::Offset(o) => format!("o:{o}"),
            Cursor::Before(id, time) => format!("b:{id}:{time}"),
        };
        URL_SAFE_NO_PAD.encode(raw)
    }

    fn decode(s: &str) -> Option<Self> {
        let raw = String::from_utf8(URL_SAFE_NO_PAD.decode(s.trim()).ok()?).ok()?;
        let mut it = raw.split(':');
        match (it.next()?, it.next(), it.next(), it.next()) {
            ("p", Some(p), Some(s), None) => {
                let page: u32 = p.parse().ok()?;
                (1..=100_000).contains(&page).then_some(Cursor::Page((page, s.parse().ok().filter(|s| *s < 10_000)?)))
            }
            ("o", Some(o), None, None) => Some(Cursor::Offset(o.parse().ok().filter(|o| *o < 10_000_000)?)),
            ("b", Some(id), Some(time), None) => Some(Cursor::Before(id.parse().ok()?, time.parse().ok().filter(|t| *t >= 0)?)),
            _ => None,
        }
    }
}

fn bad_cursor() -> ToolError {
    ToolError::invalid(
        "Invalid cursor. Pass the `next_cursor` value of the previous response unchanged, or omit `cursor` to start from the beginning.",
    )
}

pub fn page_cursor(cursor: Option<&str>) -> Result<Pos, ToolError> {
    match cursor.filter(|c| !c.trim().is_empty()) {
        None => Ok((1, 0)),
        Some(c) => match Cursor::decode(c) {
            Some(Cursor::Page(pos)) => Ok(pos),
            _ => Err(bad_cursor()),
        },
    }
}

pub fn offset_cursor(cursor: Option<&str>) -> Result<u32, ToolError> {
    match cursor.filter(|c| !c.trim().is_empty()) {
        None => Ok(0),
        Some(c) => match Cursor::decode(c) {
            Some(Cursor::Offset(o)) => Ok(o),
            _ => Err(bad_cursor()),
        },
    }
}

pub fn before_cursor(cursor: Option<&str>) -> Result<Option<(u64, i64)>, ToolError> {
    match cursor.filter(|c| !c.trim().is_empty()) {
        None => Ok(None),
        Some(c) => match Cursor::decode(c) {
            Some(Cursor::Before(id, time)) => Ok(Some((id, time))),
            _ => Err(bad_cursor()),
        },
    }
}

pub fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

pub struct Collected<T, M> {
    /// Each item with its position in the site listing.
    pub items: Vec<(T, Pos)>,
    pub next: Option<Pos>,
    pub total_items: Option<u32>,
    /// Extra data returned with the first fetched page (brand, perfumer or note header).
    pub meta: M,
}

/// Walks the site's native pages from `start` until `limit` items are gathered.
pub async fn collect_pages<T, M, F, Fut>(start: Pos, limit: u32, mut fetch: F) -> Result<Collected<T, M>, ClientError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(Listing<T>, M), ClientError>>,
{
    let (mut page, mut skip) = start;
    let (first, meta) = fetch(page).await?;
    let total_items = first.total_items;
    let mut listing = first;
    let mut items: Vec<(T, Pos)> = Vec::new();
    let mut fetched = 1;
    loop {
        let count = listing.items.len() as u32;
        let has_more_pages = match listing.total_pages {
            Some(total) => page < total,
            None => count > 0,
        };
        for (i, item) in listing.items.into_iter().enumerate().skip(skip as usize) {
            if items.len() as u32 == limit {
                return Ok(Collected { items, next: Some((page, i as u32)), total_items, meta });
            }
            items.push((item, (page, i as u32)));
        }
        if !has_more_pages {
            return Ok(Collected { items, next: None, total_items, meta });
        }
        if items.len() as u32 == limit || fetched == MAX_SITE_PAGES_PER_CALL {
            return Ok(Collected { items, next: Some((page + 1, 0)), total_items, meta });
        }
        page += 1;
        skip = 0;
        fetched += 1;
        listing = fetch(page).await?.0;
    }
}

fn json_len<T: Serialize>(v: &T) -> usize {
    serde_json::to_string(v).map(|s| s.chars().count()).unwrap_or(0)
}

/// Keeps the longest prefix of `items` whose JSON fits in `budget` characters (always at
/// least one item). Returns how many items were kept.
pub fn fit_prefix<T: Serialize>(items: &[T], budget: usize) -> usize {
    let mut used = 0;
    for (i, item) in items.iter().enumerate() {
        used += json_len(item) + 1;
        if used > budget && i > 0 {
            return i;
        }
    }
    items.len()
}

/// Applies the size cap to a page-based collection and returns `(items, next_cursor)`.
pub fn finish_pages<T: Serialize, M>(c: Collected<T, M>, budget: usize) -> (Vec<T>, Option<String>, Option<u32>, M) {
    let (mut items, positions): (Vec<T>, Vec<Pos>) = c.items.into_iter().unzip();
    let keep = fit_prefix(&items, budget);
    let next = if keep < items.len() { Some(positions[keep]) } else { c.next };
    items.truncate(keep);
    (items, next.map(|p| Cursor::Page(p).encode()), c.total_items, c.meta)
}

/// Truncates on a character boundary and reports whether anything was cut.
pub fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((idx, _)) => (s[..idx].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// Short text block: lines are appended until the digest budget is reached.
pub fn digest(header: String, lines: impl IntoIterator<Item = String>) -> String {
    let mut out = header;
    let mut omitted = 0;
    for line in lines {
        if omitted > 0 || out.chars().count() + line.chars().count() + 1 > MAX_DIGEST_CHARS {
            omitted += 1;
        } else {
            out.push('\n');
            out.push_str(&line);
        }
    }
    if omitted > 0 {
        out.push_str(&format!("\n… {omitted} more in structuredContent"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(page: u32, per_page: u32, total: u32) -> Listing<u32> {
        let start = (page - 1) * per_page;
        let items: Vec<u32> = (start..(start + per_page).min(total)).collect();
        Listing { items, page, total_pages: Some(total.div_ceil(per_page)), total_items: Some(total) }
    }

    async fn walk(start: Pos, limit: u32, per_page: u32, total: u32) -> Collected<u32, ()> {
        collect_pages(start, limit, |p| async move { Ok((listing(p, per_page, total), ())) }).await.unwrap()
    }

    #[tokio::test]
    async fn cursors_cover_every_item_exactly_once() {
        for limit in [1, 7, 20, 25, 100] {
            let mut seen = Vec::new();
            let mut pos = (1, 0);
            loop {
                let c = walk(pos, limit, 20, 53).await;
                assert!(c.items.len() as u32 <= limit);
                seen.extend(c.items.iter().map(|(v, _)| *v));
                match c.next {
                    Some(n) => pos = n,
                    None => break,
                }
            }
            assert_eq!(seen, (0..53).collect::<Vec<_>>(), "limit {limit}");
        }
    }

    #[tokio::test]
    async fn stops_after_the_page_budget_and_past_the_end() {
        let c = walk((1, 0), 100, 5, 1000).await;
        assert_eq!(c.items.len(), 25, "5 site pages of 5 items");
        assert_eq!(c.next, Some((6, 0)));
        let c = walk((4, 0), 25, 20, 53).await;
        assert!(c.items.is_empty());
        assert_eq!(c.next, None);
    }

    #[test]
    fn cursor_roundtrip_and_rejection() {
        let c = Cursor::Page((3, 12)).encode();
        assert_eq!(page_cursor(Some(&c)).unwrap(), (3, 12));
        assert_eq!(page_cursor(None).unwrap(), (1, 0));
        assert_eq!(offset_cursor(Some(&Cursor::Offset(50).encode())).unwrap(), 50);
        assert!(page_cursor(Some("garbage!")).is_err());
        assert!(page_cursor(Some(&Cursor::Offset(5).encode())).is_err(), "cursor of another kind of tool");
        assert!(offset_cursor(Some(&c)).is_err());
        assert!(page_cursor(Some(&URL_SAFE_NO_PAD.encode("p:0:0"))).is_err());
    }

    #[test]
    fn size_cap_moves_the_cursor_to_the_first_dropped_item() {
        let big = "x".repeat(1000);
        let c = Collected { items: (0..30u32).map(|i| (big.clone(), (1, i))).collect(), next: Some((2, 0)), total_items: Some(99), meta: () };
        let (items, next, total, ()) = finish_pages(c, 10_000);
        assert_eq!(items.len(), 9);
        assert_eq!(page_cursor(next.as_deref()).unwrap(), (1, 9));
        assert_eq!(total, Some(99));
        assert_eq!(fit_prefix(&["y".repeat(50_000)], 10_000), 1, "a single oversized item is still returned");
    }

    #[test]
    fn limits_and_truncation() {
        assert_eq!(clamp_limit(None), 25);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(5000)), 100);
        assert_eq!(truncate_chars("héllo", 2), ("hé".to_string(), true));
        assert_eq!(truncate_chars("abc", 3), ("abc".to_string(), false));
        let d = digest("h".into(), (0..1000).map(|i| format!("line {i} {}", "z".repeat(40))));
        assert!(d.chars().count() <= MAX_DIGEST_CHARS + 60);
        assert!(d.contains("more in structuredContent"));
    }
}
