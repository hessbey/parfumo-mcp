//! Pure HTML → model functions. No I/O. Every selector, regular expression and
//! marker comes from `site.rs`.
//!
//! Contract: a structure that should be there and is not yields
//! `ClientError::ParserOutdated("what is missing")`; never a panic, never a
//! silently empty result.

use std::collections::HashSet;

use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};

use crate::{
    error::ClientError,
    models::*,
    site::{markers, re, routes, sel},
};

type PResult<T> = Result<T, ClientError>;

mod social;
pub use social::*;

fn outdated(what: &str) -> ClientError {
    ClientError::ParserOutdated(what.to_string())
}

fn s(css: &str) -> Selector {
    // Selectors are constants of `site.rs`; a typo there is caught by the fixture tests.
    Selector::parse(css).unwrap_or_else(|_| Selector::parse("parfumo-invalid-selector").expect("static selector"))
}

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|_| Regex::new("$^").expect("static regex"))
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// All descendant text, whitespace collapsed.
fn text_of(el: ElementRef) -> String {
    squash(&el.text().collect::<String>())
}

/// Text of the direct text children only.
fn own_text(el: ElementRef) -> String {
    let mut out = String::new();
    for child in el.children() {
        if let Node::Text(t) = child.value() {
            out.push_str(t);
        }
    }
    squash(&out)
}

/// Text with line breaks preserved (`<br>`, paragraphs), skipping elements of class `skip_class`.
fn rich_text(el: ElementRef, skip_class: Option<&str>) -> String {
    fn walk(el: ElementRef, skip_class: Option<&str>, out: &mut String) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => out.push_str(&t.replace(['\n', '\r', '\t'], " ")),
                Node::Element(e) => {
                    let name = e.name();
                    if sel::TEXT_SKIP_TAGS.contains(&name) {
                        continue;
                    }
                    if let Some(skip) = skip_class
                        && e.classes().any(|c| c == skip)
                    {
                        continue;
                    }
                    if name == "br" {
                        out.push('\n');
                        continue;
                    }
                    let block = matches!(name, "p" | "div" | "li" | "blockquote");
                    if block {
                        out.push('\n');
                    }
                    if let Some(child_el) = ElementRef::wrap(child) {
                        walk(child_el, skip_class, out);
                    }
                    if block {
                        out.push('\n');
                    }
                }
                _ => {}
            }
        }
    }
    let mut raw = String::new();
    walk(el, skip_class, &mut raw);
    tidy_lines(&raw)
}

fn tidy_lines(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut blank = false;
    for line in raw.lines() {
        let line = squash(line);
        if line.is_empty() {
            blank = !lines.is_empty();
        } else {
            if blank {
                lines.push(String::new());
            }
            blank = false;
            lines.push(line);
        }
    }
    lines.join("\n")
}

fn parse_int(text: &str) -> Option<u32> {
    let caps = rx(re::FIRST_INT).captures(text)?;
    let digits: String = caps[1].chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn parse_float(text: &str) -> Option<f32> {
    squash(text).split(' ').next()?.replace(',', ".").parse().ok()
}

fn none_if_empty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

fn first<'a>(scope: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    scope.select(&s(css)).next()
}

fn attr<'a>(scope: ElementRef<'a>, css: &str, name: &str) -> Option<&'a str> {
    first(scope, css).and_then(|e| e.value().attr(name))
}

/// Path of a Parfumo URL without query or fragment (`https://host/a/b?x` → `/a/b`).
pub fn url_path(url: &str) -> &str {
    let rest = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            match after.find('/') {
                Some(j) => &after[j..],
                None => "/",
            }
        }
        None => url,
    };
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    &rest[..end]
}

/// `Brand/Perfume` id from a perfume URL or path.
pub fn perfume_id_from_url(url: &str) -> Option<String> {
    let path = url_path(url);
    let rest = path.strip_prefix(routes::PERFUMES_PREFIX)?;
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let brand = parts.next()?;
    let perfume = parts.next()?;
    Some(format!("{brand}/{perfume}"))
}

fn slug_after(url: &str, prefix: &str) -> Option<String> {
    let rest = url_path(url).strip_prefix(prefix)?;
    let slug = rest.split('/').find(|p| !p.is_empty())?;
    Some(slug.to_string())
}

fn gender_from_classes<'a>(classes: impl Iterator<Item = &'a str>) -> Option<String> {
    let classes: Vec<&str> = classes.collect();
    if classes.contains(&markers::GENDER_UNISEX) {
        Some("unisex".into())
    } else if classes.contains(&markers::GENDER_MEN) {
        Some("men".into())
    } else if classes.contains(&markers::GENDER_WOMEN) {
        Some("women".into())
    } else {
        None
    }
}

fn split_alt(alt: &str) -> (String, Option<String>) {
    match alt.rfind(markers::ALT_BY) {
        Some(i) => (squash(&alt[..i]), none_if_empty(squash(&alt[i + markers::ALT_BY.len()..]))),
        None => (squash(alt), None),
    }
}

// ---------------------------------------------------------------------------
// Page-level detection
// ---------------------------------------------------------------------------

/// True for the site's standard 404 page (also served with HTTP 404).
pub fn is_not_found_page(html: &str) -> bool {
    let head = &html[..floor_char_boundary(html, 20_000)];
    match (head.find("<title>"), head.find("</title>")) {
        (Some(a), Some(b)) if b > a => squash(&head[a + 7..b]).starts_with(markers::NOT_FOUND_TITLE),
        _ => false,
    }
}

/// True when the body is a Cloudflare challenge / Turnstile interstitial.
pub fn is_challenge(body: &str) -> bool {
    let head = &body[..floor_char_boundary(body, 60_000)];
    markers::CHALLENGE.iter().any(|m| head.contains(m))
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// `csrf_key` from the body of `/api/alive` (JSON served as `text/html`).
pub fn parse_alive(body: &str) -> PResult<String> {
    rx(re::CSRF_JSON).captures(body).map(|c| c[1].to_string()).ok_or_else(|| outdated("/api/alive did not return a csrf_key"))
}

// ---------------------------------------------------------------------------
// Perfume page
// ---------------------------------------------------------------------------

/// Tokens and counters needed to call the `/action/perfume/*` endpoints of one perfume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerfumeContext {
    pub id: String,
    pub url: String,
    pub name: String,
    pub p_id: u64,
    /// Object token for reviews, statements and similar perfumes.
    pub h: String,
    /// Object token of the classification chart.
    pub class_h: Option<String>,
    pub reviews_total: Option<u32>,
    pub statements_total: Option<u32>,
}

pub fn parse_perfume_page(html: &str) -> PResult<(PerfumeDetails, PerfumeContext)> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("perfume page".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();

    let h1 = first(root, sel::P_H1).ok_or_else(|| outdated("perfume page: h1.p_name_h1 missing"))?;
    let name = none_if_empty(own_text(h1)).ok_or_else(|| outdated("perfume page: empty perfume name"))?;
    let url = attr(root, sel::CANONICAL, "href").map(str::to_string).ok_or_else(|| outdated("perfume page: canonical link missing"))?;
    let id = perfume_id_from_url(&url).ok_or_else(|| outdated("perfume page: canonical URL is not a perfume URL"))?;

    let brand_link = first(root, sel::P_BRAND_LINK).ok_or_else(|| outdated("perfume page: brand link missing"))?;
    let brand_url = brand_link.value().attr("href").unwrap_or_default().to_string();
    let brand = EntityRef {
        id: slug_after(&brand_url, routes::PERFUMES_PREFIX).ok_or_else(|| outdated("perfume page: brand link is not a brand URL"))?,
        numeric_id: None,
        name: text_of(brand_link),
        url: brand_url,
    };

    let year = first(root, sel::P_YEAR_LINK).and_then(|e| text_of(e).parse::<u16>().ok());
    let concentration = first(root, sel::P_CONCENTRATION).and_then(|e| none_if_empty(text_of(e)));
    let gender = first(root, sel::P_GENDER_ICON).and_then(|e| gender_from_classes(e.value().classes()));

    let notes_of = |css: &str| -> Vec<String> { root.select(&s(css)).map(text_of).filter(|t| !t.is_empty()).collect() };
    let notes = NotePyramid {
        top: notes_of(sel::P_NOTES_TOP),
        heart: notes_of(sel::P_NOTES_HEART),
        base: notes_of(sel::P_NOTES_BASE),
        flat: notes_of(sel::P_NOTES_FLAT),
    };

    let mut seen = HashSet::new();
    let perfumers: Vec<EntityRef> = root
        .select(&s(sel::P_PERFUMERS))
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let slug = slug_after(href, routes::PERFUMERS_PREFIX)?;
            let name = text_of(a);
            if name.is_empty() || !seen.insert(slug.clone()) {
                return None;
            }
            Some(EntityRef { id: slug, numeric_id: None, name, url: href.to_string() })
        })
        .collect();

    let mut accords: Vec<String> = Vec::new();
    for el in root.select(&s(sel::P_ACCORDS)) {
        let t = text_of(el);
        if !t.is_empty() && !accords.contains(&t) {
            accords.push(t);
        }
    }

    let mut ratings = Vec::new();
    let mut p_id: Option<u64> = None;
    let mut h: Option<String> = None;
    let h_re = rx(re::H_TOKEN);
    for item in root.select(&s(sel::P_RATING_ITEM)) {
        let v = item.value();
        if p_id.is_none() {
            p_id = v.attr("data-p_id").and_then(|x| x.parse().ok());
        }
        if h.is_none() {
            h = v.attr("data-h").filter(|x| h_re.is_match(x)).map(str::to_string);
        }
        let (Some(dimension), Some(score)) = (v.attr("data-type"), first(item, sel::P_RATING_SCORE)) else {
            continue;
        };
        if let Some(average) = parse_float(&text_of(score)) {
            ratings.push(RatingScore {
                dimension: dimension.to_string(),
                average,
                votes: first(item, sel::P_RATING_COUNT).and_then(|e| parse_int(&text_of(e))),
            });
        }
    }

    let mut reviews_total = None;
    let mut statements_total = None;
    if let Some(c) = rx(re::REVIEWS_CALL).captures(html) {
        reviews_total = c[1].parse().ok();
        p_id = p_id.or_else(|| c[2].parse().ok());
        h = h.or_else(|| Some(c[3].to_string()));
    }
    if let Some(c) = rx(re::STATEMENTS_CALL).captures(html) {
        statements_total = c[1].parse().ok();
        p_id = p_id.or_else(|| c[2].parse().ok());
        h = h.or_else(|| Some(c[3].to_string()));
    }
    let mut class_h = None;
    if let Some(c) = rx(re::CLASS_CALL).captures(html) {
        p_id = p_id.or_else(|| c[1].parse().ok());
        class_h = Some(c[2].to_string());
    }
    if p_id.is_none() {
        p_id = rx(re::P_ID_FALLBACK).captures(html).and_then(|c| c[1].parse().ok());
    }
    let p_id = p_id.ok_or_else(|| outdated("perfume page: numeric p_id not found"))?;
    let h = h.ok_or_else(|| outdated("perfume page: object token h not found"))?;

    let description = first(root, sel::P_DESCRIPTION).and_then(|e| none_if_empty(squash(&rich_text(e, Some(sel::P_DESCRIPTION_SKIP_CLASS)))));

    let details = PerfumeDetails {
        id: id.clone(),
        p_id: Some(p_id),
        name: name.clone(),
        brand,
        year,
        gender,
        concentration,
        perfumers,
        notes,
        accords,
        ratings,
        classification: Vec::new(),
        ranking: first(root, sel::P_RANK_LABEL).and_then(|e| none_if_empty(text_of(e))),
        facts: first(root, sel::P_FACTS).and_then(|e| none_if_empty(rich_text(e, None))),
        description,
        reviews_count: reviews_total,
        statements_count: statements_total,
        image_url: attr(root, sel::OG_IMAGE, "content").map(str::to_string),
        url: url.clone(),
        warnings: Vec::new(),
    };
    let ctx = PerfumeContext { id, url, name, p_id, h, class_h, reviews_total, statements_total };
    Ok((details, ctx))
}

/// Community classification from the `get_classification_chart.php` fragment
/// (percentages are read from the `aria-label` of each chart). Empty when the site
/// shows its "not classified yet" placeholder.
pub fn parse_classification(fragment: &str) -> PResult<Vec<ClassGroup>> {
    let doc = Html::parse_fragment(fragment);
    let pair = rx(re::PERCENT_PAIR);
    let mut groups = Vec::new();
    for col in doc.root_element().select(&s(sel::CLASS_COL)) {
        let (Some(title), Some(svg)) = (first(col, sel::CLASS_TITLE), first(col, sel::CLASS_SVG)) else {
            continue;
        };
        let label = svg.value().attr("aria-label").unwrap_or_default();
        let shares: Vec<ClassShare> = label
            .split(',')
            .filter_map(|part| {
                let c = pair.captures(part)?;
                Some(ClassShare { label: c[1].to_string(), percent: c[2].parse().ok()? })
            })
            .collect();
        if shares.is_empty() {
            continue;
        }
        groups.push(ClassGroup { group: text_of(title), votes: first(col, sel::CLASS_VOTES).and_then(|e| parse_int(&text_of(e))), shares });
    }
    if groups.is_empty() && first(doc.root_element(), sel::CLASS_EMPTY).is_none() {
        return Err(outdated("classification chart: no chart with percentages found"));
    }
    Ok(groups)
}

// ---------------------------------------------------------------------------
// Reviews and statements
// ---------------------------------------------------------------------------

fn member_ratings(scope: ElementRef) -> MemberRatings {
    let mut out = MemberRatings::default();
    for item in scope.select(&s(sel::VOTING_ITEM)) {
        let Some(nr) = first(item, sel::VOTING_NR) else { continue };
        let Some(value) = parse_float(&text_of(nr)) else { continue };
        let label = own_text(item).to_lowercase();
        let Some((_, scale)) = markers::VOTE_LABELS.iter().find(|(needle, _)| label.contains(needle)) else {
            continue;
        };
        match *scale {
            "scent" => out.scent = Some(value),
            "durability" => out.durability = Some(value),
            "sillage" => out.sillage = Some(value),
            "bottle" => out.bottle = Some(value),
            _ => out.pricing = Some(value),
        }
    }
    out
}

/// Reviews from the perfume page or from the `get_reviews.php` fragment.
/// `perfume_url` is used to build the review URL when the fragment has no link.
pub fn parse_reviews(html: &str, perfume_url: &str) -> PResult<Vec<Review>> {
    let doc = Html::parse_fragment(html);
    let class_id = rx(re::REVIEW_CLASS_ID);
    let trailing = rx(re::TRAILING_ID);
    let mut out = Vec::new();
    for article in doc.root_element().select(&s(sel::REVIEW)) {
        let id = article
            .value()
            .attr("class")
            .and_then(|c| class_id.captures(c))
            .or_else(|| attr(article, sel::REVIEW_HEADER, "id").and_then(|i| trailing.captures(i)))
            .and_then(|c| c[1].parse::<u64>().ok())
            .ok_or_else(|| outdated("review: numeric id missing"))?;
        let text_el = first(article, sel::REVIEW_TEXT).ok_or_else(|| outdated("review: text block missing"))?;
        let author = first(article, sel::REVIEW_AUTHOR).map(text_of).filter(|a| !a.is_empty()).ok_or_else(|| outdated("review: author missing"))?;
        let url = attr(article, sel::REVIEW_LINK, "href")
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}/reviews/{id}", perfume_url.trim_end_matches('/')));
        let date = first(article, sel::REVIEW_DATE).and_then(|e| e.value().attr("content").map(str::to_string).or_else(|| none_if_empty(text_of(e))));
        out.push(Review {
            id,
            url,
            author,
            author_url: attr(article, sel::REVIEW_AUTHOR_LINK, "href").map(str::to_string),
            date,
            title: first(article, sel::REVIEW_TITLE).and_then(|e| none_if_empty(text_of(e))),
            text: rich_text(text_el, None),
            truncated: first(text_el, sel::REVIEW_TRUNCATED).is_some(),
            ratings: member_ratings(article),
            awards: first(article, sel::REVIEW_AWARDS).and_then(|e| parse_int(&text_of(e))),
            comments: first(article, sel::REVIEW_COMMENTS).and_then(|e| parse_int(&text_of(e))),
        });
    }
    if out.is_empty() && !html.trim().is_empty() {
        return Err(outdated("reviews fragment: no article.review found"));
    }
    Ok(out)
}

/// Statements from the perfume page or from the `get_statements.php` fragment.
pub fn parse_statements(html: &str, perfume_url: &str) -> PResult<Vec<Statement>> {
    let doc = Html::parse_fragment(html);
    let trailing = rx(re::TRAILING_ID);
    let mut out = Vec::new();
    for block in doc.root_element().select(&s(sel::STATEMENT)) {
        let id = block
            .value()
            .attr("id")
            .and_then(|i| trailing.captures(i))
            .and_then(|c| c[1].parse::<u64>().ok())
            .ok_or_else(|| outdated("statement: numeric id missing"))?;
        let text_el = first(block, sel::STATEMENT_TEXT).ok_or_else(|| outdated("statement: text block missing"))?;
        let author_link = first(block, sel::STATEMENT_AUTHOR_LINK).ok_or_else(|| outdated("statement: author link missing"))?;
        let author = none_if_empty(text_of(author_link))
            .or_else(|| attr(author_link, "img", "alt").map(str::to_string))
            .ok_or_else(|| outdated("statement: author missing"))?;
        let author_url = author_link.value().attr("href").map(|href| {
            let path_end = href.find(routes::USERS_PREFIX).map(|i| i + routes::USERS_PREFIX.len());
            match path_end.and_then(|start| href[start..].find('/').map(|j| start + j)) {
                Some(cut) => href[..cut].to_string(),
                None => href.to_string(),
            }
        });
        let url = attr(block, sel::STATEMENT_LINK, "href")
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}/statements/{id}", perfume_url.trim_end_matches('/')));
        out.push(Statement {
            id,
            url,
            author,
            author_url,
            date: first(block, sel::STATEMENT_DATE).and_then(|e| none_if_empty(text_of(e))),
            text: rich_text(text_el, None),
            ratings: member_ratings(block),
            awards: first(block, sel::STATEMENT_AWARDS).and_then(|e| parse_int(&text_of(e))),
            comments: first(block, sel::STATEMENT_COMMENTS).and_then(|e| parse_int(&text_of(e))),
        });
    }
    if out.is_empty() && !html.trim().is_empty() {
        return Err(outdated("statements fragment: no div.statement found"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Similar perfumes and dupes
// ---------------------------------------------------------------------------

/// Items of the `get_similar.php` / `get_inspiration.php` fragments. The site gives no
/// link for these items, only the numeric id.
pub fn parse_similar(fragment: &str) -> PResult<Vec<SimilarPerfume>> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let mut out = Vec::new();
    for item in root.select(&s(sel::SIM_ITEM)) {
        let p_id =
            item.value().attr("data-s_id").and_then(|x| x.parse::<u64>().ok()).ok_or_else(|| outdated("similar item: data-s_id is not numeric"))?;
        let img = first(item, sel::SIM_IMG);
        let alt = img.and_then(|i| i.value().attr("alt")).unwrap_or_default();
        let (alt_name, brand) = split_alt(alt);
        let name_el = first(item, sel::SIM_NAME);
        let name = name_el.map(own_text).filter(|n| !n.is_empty()).unwrap_or(alt_name);
        if name.is_empty() {
            return Err(outdated("similar item: name missing"));
        }
        out.push(SimilarPerfume {
            p_id,
            name,
            brand,
            concentration: first(item, sel::SIM_CONCENTRATION).and_then(|e| none_if_empty(text_of(e))),
            percent: attr(item, sel::SIM_FILL, "data-percentage").and_then(|p| p.parse().ok()),
            votes: None,
            id: None,
            url: None,
            image_url: img.and_then(|i| i.value().attr("src")).map(str::to_string),
        });
    }
    if out.is_empty() && !fragment.trim().is_empty() && first(root, sel::SIM_HOLDER).is_none() {
        return Err(outdated("similar fragment: no .sim_item found"));
    }
    Ok(out)
}

/// Link and vote count from the `similar_sneak_peek.php` fragment.
pub fn parse_sneak_peek(fragment: &str) -> PResult<(String, Option<u32>)> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let url = attr(root, sel::PEEK_LINK, "href")
        .filter(|u| perfume_id_from_url(u).is_some())
        .ok_or_else(|| outdated("similar sneak peek: perfume link missing"))?;
    Ok((url.to_string(), first(root, sel::PEEK_VOTES).and_then(|e| parse_int(&text_of(e)))))
}

fn strip_dash(text: String) -> Option<String> {
    none_if_empty(squash(text.trim_start_matches([' ', '-', '–'])))
}

/// Groups of the dupes directory from the `_get_dupes.php` fragment.
pub fn parse_dupes(fragment: &str) -> PResult<Vec<DupeGroup>> {
    let doc = Html::parse_fragment(fragment);
    let mut out = Vec::new();
    for item in doc.root_element().select(&s(sel::DUPE_ITEM)) {
        let link = first(item, sel::DUPE_MAIN_LINK).ok_or_else(|| outdated("dupes: original link missing"))?;
        let url = link.value().attr("href").unwrap_or_default().to_string();
        let name_el = first(item, sel::DUPE_MAIN_NAME).ok_or_else(|| outdated("dupes: original name missing"))?;
        let original = PerfumeSummary {
            id: perfume_id_from_url(&url).ok_or_else(|| outdated("dupes: original URL is not a perfume URL"))?,
            name: own_text(name_el),
            brand: first(name_el, sel::DUPE_MAIN_BRAND).and_then(|e| strip_dash(text_of(e))),
            concentration: first(name_el, sel::LABEL_A).and_then(|e| none_if_empty(text_of(e))),
            image_url: attr(item, sel::DUPE_MAIN_IMG, "src").map(str::to_string),
            url,
            ..Default::default()
        };
        let mut dupes = Vec::new();
        for d in item.select(&s(sel::DUPE_DUPE)) {
            let Some(a) = first(d, sel::DUPE_DUPE_LINK) else { continue };
            let url = a.value().attr("href").unwrap_or_default().to_string();
            let Some(id) = perfume_id_from_url(&url) else { continue };
            let Some(name_el) = first(d, sel::DUPE_DUPE_NAME) else { continue };
            dupes.push(DupeCandidate {
                perfume: PerfumeSummary {
                    id,
                    name: own_text(name_el),
                    brand: first(d, sel::DUPE_DUPE_BRAND).and_then(|e| strip_dash(text_of(e))),
                    concentration: first(name_el, sel::LABEL_A).and_then(|e| none_if_empty(text_of(e))),
                    url,
                    ..Default::default()
                },
                votes: first(d, sel::DUPE_DUPE_VOTES).and_then(|e| parse_int(&text_of(e))),
            });
        }
        if original.name.is_empty() {
            return Err(outdated("dupes: empty original name"));
        }
        out.push(DupeGroup { original, dupes });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Listings
// ---------------------------------------------------------------------------

fn parse_col(col: ElementRef) -> Option<PerfumeSummary> {
    let link = first(col, sel::COL_LINK)?;
    let url = link.value().attr("href")?.to_string();
    let id = perfume_id_from_url(&url)?;
    let img = first(col, sel::COL_IMG);
    let (alt_name, alt_brand) = split_alt(img.and_then(|i| i.value().attr("alt")).unwrap_or_default());
    let name = none_if_empty(own_text(link)).unwrap_or(alt_name);
    if name.is_empty() {
        return None;
    }
    Some(PerfumeSummary {
        id,
        p_id: None,
        name,
        brand: first(col, sel::COL_BRAND).and_then(|e| none_if_empty(text_of(e))).or(alt_brand),
        year: first(col, sel::COL_YEAR).and_then(|e| rx(re::YEAR).captures(&text_of(e)).and_then(|c| c[1].parse().ok())),
        concentration: first(link, sel::LABEL_A).and_then(|e| none_if_empty(text_of(e))),
        rating: first(col, sel::COL_RATING).and_then(|e| parse_float(&own_text(e))),
        rating_count: first(col, sel::COL_RATING_COUNT).and_then(|e| parse_int(&text_of(e))),
        gender: first(col, sel::COL_GENDER_ICON).and_then(|e| gender_from_classes(e.value().classes())),
        reviews_count: attr(col, sel::COL_REVIEWS, "title").and_then(parse_int),
        trend_count: first(col, sel::COL_TREND_COUNT).and_then(|e| parse_int(&text_of(e))),
        rank: first(col, sel::COL_PLACE).and_then(|e| parse_int(&text_of(e))),
        url,
        image_url: img.and_then(|i| i.value().attr("src")).map(str::to_string),
    })
}

/// `(first, last, total)` of the "1 - 20 by 344" counter and the highest page link.
fn paging_marks(root: ElementRef) -> (Option<(u32, u32, u32)>, Option<u32>) {
    let count_re = rx(re::PAGING_COUNT);
    let digits = |t: &str| t.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok();
    let range = root.select(&s(sel::PAGING_COUNT)).find_map(|e| {
        let text = text_of(e);
        let c = count_re.captures(&text)?;
        Some((digits(&c[1])?, digits(&c[2])?, digits(&c[3])?))
    });
    let page_re = rx(re::CURRENT_PAGE);
    let max_link =
        root.select(&s(sel::PAGE_LINKS)).filter_map(|a| page_re.captures(a.value().attr("href")?)?.get(1)?.as_str().parse::<u32>().ok()).max();
    (range, max_link)
}

/// Totals of a paginated page; empties `items` when the page asked for is past the end.
fn page_bounds<T>(range: Option<(u32, u32, u32)>, max_link: Option<u32>, requested_page: u32, items: &mut Vec<T>) -> (Option<u32>, Option<u32>) {
    match range {
        Some((first_idx, last_idx, total)) => {
            // The site answers an out-of-range page with page 1: detect it from the range.
            if requested_page > 1 && first_idx <= 1 {
                items.clear();
            }
            let per_page = (last_idx + 1).saturating_sub(first_idx).max(1);
            let shown = total.min(routes::MAX_LISTED_RESULTS);
            let pages = max_link.unwrap_or(1).max(if first_idx <= 1 { shown.div_ceil(per_page) } else { requested_page });
            (Some(total), Some(pages))
        }
        None => match max_link {
            Some(max) => {
                if requested_page > max {
                    items.clear();
                }
                (None, Some(max))
            }
            None => {
                // Single page without any pagination block.
                if requested_page > 1 {
                    items.clear();
                }
                (Some(items.len() as u32), Some(1))
            }
        },
    }
}

fn listing_from(root: ElementRef, requested_page: u32) -> PResult<Listing<PerfumeSummary>> {
    let requested_page = requested_page.max(1);
    let cols: Vec<ElementRef> = root.select(&s(sel::GRID_COL)).collect();
    let (range, max_link) = paging_marks(root);
    if cols.is_empty() {
        // Page just past the end: empty grid, but the counter is still there ("161 - 151 by 151").
        if let (Some((_, _, total)), true) = (range, first(root, sel::GRID).is_some() && requested_page > 1) {
            return Ok(Listing { items: Vec::new(), page: requested_page, total_pages: max_link, total_items: Some(total) });
        }
        if first(root, sel::NOTHING_FOUND).is_some() {
            // On page 1 this means "no result"; further pages only tell that the page is out of range.
            let total = if requested_page == 1 { Some(0) } else { None };
            return Ok(Listing { items: Vec::new(), page: requested_page, total_pages: total, total_items: total });
        }
        return Err(outdated("listing: no .pgrid > .col item and no 'nothing found' marker"));
    }
    let mut items = Vec::with_capacity(cols.len());
    for col in cols {
        items.push(parse_col(col).ok_or_else(|| outdated("listing item: perfume link or name missing"))?);
    }

    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

/// One page of a perfume listing: search results, brand, perfumer, note, top lists,
/// recently added, release year.
pub fn parse_listing(html: &str, requested_page: u32) -> PResult<Listing<PerfumeSummary>> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("listing page".into()));
    }
    let doc = Html::parse_document(html);
    listing_from(doc.root_element(), requested_page)
}

/// Sections of `/Perfumes/Trends`: (title, ranked perfumes).
pub fn parse_trends(html: &str) -> PResult<Vec<(String, Vec<PerfumeSummary>)>> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for grid in doc.root_element().select(&s(sel::GRID)) {
        let title = grid.parent().and_then(ElementRef::wrap).and_then(|p| first(p, sel::SECTION_TITLE)).map(text_of).unwrap_or_default();
        let items: Vec<PerfumeSummary> = grid.select(&s(sel::COL)).filter_map(parse_col).collect();
        // A trend section is a titled grid of ranked items; other grids are ignored.
        if !title.is_empty() && items.first().is_some_and(|i| i.rank.is_some()) {
            out.push((title, items));
        }
    }
    if out.is_empty() {
        return Err(outdated("trends page: no ranked section found"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Brand, perfumer, note
// ---------------------------------------------------------------------------

struct EntityHead {
    name: String,
    url: String,
    country: Option<String>,
    founded: Option<u16>,
}

fn entity_head(root: ElementRef, what: &str) -> PResult<EntityHead> {
    let h1 = first(root, sel::ENTITY_H1).ok_or_else(|| outdated(&format!("{what} page: h1 missing")))?;
    // Page 1 has the name as plain text, the following pages wrap it in a link.
    let name = none_if_empty(own_text(h1))
        .or_else(|| first(h1, sel::ENTITY_H1_LINK).and_then(|a| none_if_empty(text_of(a))))
        .ok_or_else(|| outdated(&format!("{what} page: empty name")))?;
    let url = attr(root, sel::CANONICAL, "href")
        .or_else(|| attr(root, sel::ENTITY_SORT_FORM, "action"))
        .map(|u| u.split('?').next().unwrap_or(u).to_string())
        .ok_or_else(|| outdated(&format!("{what} page: canonical URL missing")))?;
    let mut country = None;
    let mut founded = None;
    let since = rx(re::SINCE_YEAR);
    for tag in root.select(&s(sel::ENTITY_TAGS)) {
        let text = text_of(tag);
        if first(tag, sel::ENTITY_FLAG).is_some() {
            country = none_if_empty(text);
        } else if let Some(c) = since.captures(&text) {
            founded = c[1].parse().ok();
        }
    }
    Ok(EntityHead { name, url, country, founded })
}

pub fn parse_brand_page(html: &str, requested_page: u32) -> PResult<BrandPage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("brand page".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let head = entity_head(root, "brand")?;
    let id = slug_after(&head.url, routes::PERFUMES_PREFIX).ok_or_else(|| outdated("brand page: URL is not a brand URL"))?;
    Ok(BrandPage {
        brand: EntityRef { id, numeric_id: rx(re::BRAND_ID).captures(html).and_then(|c| c[1].parse().ok()), name: head.name, url: head.url },
        country: head.country,
        founded: head.founded,
        perfumes: listing_from(root, requested_page)?,
    })
}

pub fn parse_perfumer_page(html: &str, requested_page: u32) -> PResult<PerfumerPage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("perfumer page".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let head = entity_head(root, "perfumer")?;
    let id = slug_after(&head.url, routes::PERFUMERS_PREFIX).ok_or_else(|| outdated("perfumer page: URL is not a perfumer URL"))?;
    Ok(PerfumerPage {
        perfumer: EntityRef { id, numeric_id: rx(re::PERFUMER_ID).captures(html).and_then(|c| c[1].parse().ok()), name: head.name, url: head.url },
        country: head.country,
        perfumes: listing_from(root, requested_page)?,
    })
}

pub fn parse_note_page(html: &str, requested_page: u32) -> PResult<NotePage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("fragrance note page".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let head = entity_head(root, "note")?;
    let (id, is_category) = match slug_after(&head.url, routes::NOTE_CATEGORY_PREFIX) {
        Some(slug) => (slug, true),
        None => (slug_after(&head.url, routes::NOTE_PREFIX).ok_or_else(|| outdated("note page: URL is not a note URL"))?, false),
    };
    let mut seen = HashSet::new();
    let main = sel::MAIN_CANDIDATES.iter().find_map(|c| first(root, c)).unwrap_or(root);
    let categories: Vec<EntityRef> = main
        .select(&s(sel::NOTE_CATEGORY_LINKS))
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let slug = slug_after(href, routes::NOTE_CATEGORY_PREFIX)?;
            let name = text_of(a);
            if name.is_empty() || slug == "scentmix" || (is_category && slug == id) || !seen.insert(slug.clone()) {
                return None;
            }
            Some(EntityRef { id: slug, numeric_id: None, name, url: href.split('?').next()?.to_string() })
        })
        .collect();
    Ok(NotePage {
        note: EntityRef { id, numeric_id: rx(re::NOTE_ID).captures(html).and_then(|c| c[1].parse().ok()), name: head.name, url: head.url },
        is_category,
        categories,
        perfumes: listing_from(root, requested_page)?,
    })
}

// ---------------------------------------------------------------------------
// Autocompletion fragments
// ---------------------------------------------------------------------------

/// `(numeric id, name)` pairs of a `label_filters_*.php` fragment. `id_attr` is the
/// data attribute carrying the id (`data-b_id`, `data-pf_id`, `data-n_inc_id`).
pub fn parse_label_filter(fragment: &str, id_attr: &str) -> Vec<(u64, String)> {
    let doc = Html::parse_fragment(fragment);
    doc.root_element()
        .select(&s(sel::LABEL_FILTER_CELL))
        .filter_map(|td| {
            let id = td.value().attr(id_attr)?.parse().ok()?;
            Some((id, squash(td.value().attr("data-name")?)))
        })
        .collect()
}

/// `(name, url)` pairs of the brand / perfumer livesearch fragments.
pub fn parse_livesearch_links(fragment: &str) -> Vec<(String, String)> {
    let doc = Html::parse_fragment(fragment);
    doc.root_element()
        .select(&s(sel::LIVESEARCH_ITEM))
        .filter_map(|item| {
            let name = first(item, sel::LIVESEARCH_ITEM_NAME).map(text_of).filter(|n| !n.is_empty())?;
            Some((name, attr(item, sel::LIVESEARCH_ITEM_LINK, "href")?.to_string()))
        })
        .collect()
}

/// `(name, slug)` pairs of the note livesearch fragment.
pub fn parse_livesearch_notes(fragment: &str) -> Vec<(String, String)> {
    let doc = Html::parse_fragment(fragment);
    doc.root_element()
        .select(&s(sel::LIVESEARCH_NOTE))
        .filter_map(|el| {
            let slug = el.value().attr("data-name_url").filter(|v| !v.is_empty())?;
            Some((text_of(el), slug.to_string()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Generic page text (fetch_page)
// ---------------------------------------------------------------------------

fn page_text(el: ElementRef, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(&t.replace(['\n', '\r', '\t'], " ")),
            Node::Element(e) => {
                let name = e.name();
                if sel::TEXT_SKIP_TAGS.contains(&name)
                    || e.attr("id").is_some_and(|id| sel::TEXT_SKIP_IDS.contains(&id))
                    || e.classes().any(|c| c == "hide")
                {
                    continue;
                }
                let block = sel::TEXT_BLOCK_TAGS.contains(&name);
                if block {
                    out.push('\n');
                }
                if name == "img" {
                    continue;
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    page_text(child_el, out);
                }
                if block {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            _ => {}
        }
    }
}

/// Cleaned text, title and same-site links of an arbitrary page.
/// `url` is the absolute URL that was fetched; `max_links` caps the link list.
pub fn parse_page_text(html: &str, url: &str, max_links: usize) -> PResult<FetchedPage> {
    if html.trim().is_empty() {
        return Err(ClientError::Upstream("empty page body".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let main = sel::MAIN_CANDIDATES.iter().find_map(|c| first(root, c)).ok_or_else(|| outdated("page: no body element"))?;
    let mut raw = String::new();
    page_text(main, &mut raw);
    let text: String = tidy_lines(&raw).lines().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");

    let origin = match url.find("://").and_then(|i| url[i + 3..].find('/').map(|j| i + 3 + j)) {
        Some(end) => &url[..end],
        None => url,
    };
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for a in main.select(&s(sel::LINKS)) {
        if links.len() >= max_links {
            break;
        }
        let Some(href) = a.value().attr("href") else { continue };
        let absolute = if href.starts_with('/') && !href.starts_with("//") {
            format!("{origin}{href}")
        } else if href.starts_with(origin) {
            href.to_string()
        } else {
            continue;
        };
        if absolute.contains(routes::LOGIN_REDIRECT_MARKER) {
            continue;
        }
        let label = text_of(a);
        if label.is_empty() || !seen.insert(absolute.clone()) {
            continue;
        }
        links.push(PageLink { text: label, url: absolute });
    }
    Ok(FetchedPage { url: url.to_string(), title: first(root, sel::TITLE).and_then(|e| none_if_empty(text_of(e))), text, links })
}

// ---------------------------------------------------------------------------
// Community reads: blogs, Souk, wall — jalon 4
// ---------------------------------------------------------------------------

/// Number written next to an icon (`<div><i class="fa fa-trophy"></i> 3</div>`).
fn icon_count(scope: ElementRef, icon_css: &str) -> Option<u32> {
    let parent = first(scope, icon_css)?.parent().and_then(ElementRef::wrap)?;
    parse_int(&text_of(parent))
}

fn blog_cards(root: ElementRef) -> PResult<Vec<BlogEntry>> {
    let url_re = rx(re::BLOG_ARTICLE_URL);
    let mut items = Vec::new();
    for card in root.select(&s(sel::BLOG_CARD)) {
        let link = first(card, sel::BLOG_CARD_LINK).and_then(|a| a.value().attr("href"));
        let ids = link.and_then(|href| url_re.captures(href));
        let title = first(card, sel::BLOG_CARD_TITLE).map(text_of).and_then(none_if_empty);
        let (Some(url), Some(ids), Some(title)) = (link, ids, title) else { return Err(outdated("blog list: article without link or title")) };
        // The profile URL is the article URL cut before `/Blog/…`.
        let profile = &url[..url.find("/Blog/").unwrap_or(url.len())];
        items.push(BlogEntry {
            id: format!("{}/{}", &ids[1], &ids[2]),
            url: url.to_string(),
            title,
            author: ids[1].to_string(),
            author_url: attr(card, sel::BLOG_CARD_AUTHOR, "href").unwrap_or(profile).to_string(),
            date: first(card, sel::BLOG_CARD_DATE).map(text_of).and_then(none_if_empty),
            text: first(card, sel::BLOG_CARD_EXCERPT).map(text_of).and_then(none_if_empty),
            comments: icon_count(card, sel::BLOG_CARD_COMMENTS_ICON),
            awards: icon_count(card, sel::BLOG_CARD_AWARDS_ICON),
            image_url: attr(card, sel::BLOG_CARD_COVER, "src").map(str::to_string),
        });
    }
    Ok(items)
}

fn finish_blog_list(root: ElementRef, requested_page: u32, mut items: Vec<BlogEntry>, what: &str) -> PResult<Listing<BlogEntry>> {
    if items.is_empty() && first(root, sel::NOTHING_FOUND).is_none() && requested_page == 1 {
        return Err(outdated(&format!("{what}: neither an article nor the 'nothing yet' marker")));
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

/// One page of a site-wide blog feed (`/blogs`, `/blogs/news`: 8 per page; `/blogs/top`,
/// `/blogs/hot`: the week's articles on a single page).
pub fn parse_blog_feed(html: &str, requested_page: u32) -> PResult<Listing<BlogEntry>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::BLOG_FEED_TABS).is_none() {
        return Err(outdated("blog feed: tabs not found"));
    }
    let items = blog_cards(root)?;
    finish_blog_list(root, requested_page, items, "blog feed")
}

/// One page of `/Users/{nick}/Blog` (6 articles per page).
pub fn parse_user_blog(html: &str, requested_page: u32) -> PResult<Listing<BlogEntry>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = user_text_page(&doc, "blog")?;
    let items = blog_cards(root)?;
    finish_blog_list(root, requested_page, items, "member blog page")
}

fn comment_fields(el: ElementRef) -> Option<(String, String, Option<String>, String)> {
    let author = first(el, sel::COMMENT_AUTHOR)?;
    let text = first(el, sel::COMMENT_TEXT)?;
    Some((
        none_if_empty(text_of(author))?,
        author.value().attr("href")?.to_string(),
        first(el, sel::COMMENT_DATE).map(text_of).and_then(none_if_empty),
        rich_text(text, None),
    ))
}

/// Comments of a wall or of an article, with their answers nested.
fn comment_entries(scope: ElementRef) -> PResult<Vec<WallEntry>> {
    let trailing = rx(re::TRAILING_ID);
    let mut entries = Vec::new();
    for holder in scope.select(&s(sel::COMMENT)) {
        let id = holder.value().attr("data-comment_id").and_then(|v| v.trim().parse::<u64>().ok());
        // The first author, date and text of the holder are its own: answers come after them.
        let (Some(id), Some((author, author_url, date, text))) = (id, comment_fields(holder)) else {
            return Err(outdated("comment: id, author or text missing"));
        };
        let mut replies = Vec::new();
        for reply in holder.select(&s(sel::COMMENT_REPLY)) {
            let reply_id = reply.value().attr("id").and_then(|i| trailing.captures(i)).and_then(|c| c[1].parse::<u64>().ok());
            let (Some(id), Some((author, author_url, date, text))) = (reply_id, comment_fields(reply)) else {
                return Err(outdated("comment reply: id, author or text missing"));
            };
            replies.push(WallReply { id, author, author_url, date, text });
        }
        entries.push(WallEntry { id, author, author_url, date, text, replies });
    }
    Ok(entries)
}

/// A blog article page. `url` is the absolute URL it was read from (the page has no canonical link).
pub fn parse_blog_article(html: &str, url: &str) -> PResult<BlogArticle> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let Some(article) = first(root, sel::BLOG_ARTICLE) else { return Err(outdated("blog article: article block not found")) };
    let ids = rx(re::BLOG_ARTICLE_URL).captures(url).ok_or_else(|| outdated("blog article: unexpected article URL"))?;
    let title = first(article, sel::BLOG_ARTICLE_TITLE).map(text_of).and_then(none_if_empty);
    let text_el = first(article, sel::BLOG_ARTICLE_TEXT);
    let (Some(title), Some(text_el)) = (title, text_el) else { return Err(outdated("blog article: title or text not found")) };
    let blog_links: Vec<ElementRef> = article.select(&s(sel::BLOG_ARTICLE_BLOG_LINK)).collect();
    let author_url = blog_links
        .first()
        .and_then(|a| a.value().attr("href"))
        .map(|href| href.trim_end_matches("/Blog").to_string())
        .unwrap_or_else(|| url[..url.find("/Blog/").unwrap_or(url.len())].to_string());
    Ok(BlogArticle {
        id: format!("{}/{}", &ids[1], &ids[2]),
        article_id: article.value().attr("id").and_then(|i| rx(re::TRAILING_ID).captures(i)).and_then(|c| c[1].parse().ok()),
        url: url.to_string(),
        title,
        author: ids[1].to_string(),
        author_url,
        author_id: rx(re::BLOG_AUTHOR_ID).captures(html).and_then(|c| c[1].parse().ok()),
        blog_title: blog_links.iter().map(|a| text_of(*a)).find(|t| !t.is_empty()),
        date: first(article, sel::BLOG_ARTICLE_DATE).map(text_of).and_then(none_if_empty),
        text: rich_text(text_el, None),
        images: article.select(&s(sel::BLOG_ARTICLE_IMAGES)).filter_map(|i| i.value().attr("src")).map(str::to_string).collect(),
        comments: first(article, sel::BLOG_ARTICLE_COMMENTS).and_then(|e| parse_int(&text_of(e))),
        awards: first(article, sel::BLOG_ARTICLE_AWARDS).and_then(|e| parse_int(&text_of(e))),
        comment_list: comment_entries(article)?,
    })
}

/// One page of `/Users/{nick}/Wall` (20 entries per page, answers nested). The numeric id
/// of the owner is read from the "Post to Wall" button.
pub fn parse_wall(html: &str, requested_page: u32) -> PResult<WallPage> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = user_text_page(&doc, "wall")?;
    let owner = first(root, sel::USER_PAGE_HEAD).map(text_of).and_then(none_if_empty);
    let owner_url = attr(root, sel::MEMBER_NAV_PROFILE, "href");
    let (Some(owner), Some(owner_url)) = (owner, owner_url) else { return Err(outdated("wall page: owner nickname or profile link not found")) };
    let owner_id = rx(re::WALL_OWNER_ID).captures(html).and_then(|c| c[1].parse::<u64>().ok());
    let mut items = comment_entries(root)?;
    // An empty wall still shows the "Post to Wall" button (owner id) or the empty-state picture.
    if items.is_empty() && owner_id.is_none() && first(root, sel::NOTHING_FOUND).is_none() && requested_page == 1 {
        return Err(outdated("wall page: neither an entry, the post button nor the 'nothing yet' marker"));
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(WallPage { owner, owner_url: owner_url.to_string(), owner_id, entries: Listing { items, page: requested_page, total_pages, total_items } })
}

fn flag_code(scope: ElementRef, css: &str) -> Option<String> {
    let src = attr(scope, css, "src")?;
    Some(rx(re::FLAG_CODE).captures(src)?[1].to_ascii_lowercase())
}

/// A card of a Souk list (site-wide list, search result, member's Souk).
fn souk_card(col: ElementRef) -> Option<SoukItem> {
    let url = attr(col, sel::SOUK_CARD_LINK, "href")?;
    let ids = rx(re::SOUK_ITEM_URL).captures(url)?;
    let kind = if &ids[2] == markers::SOUK_SHARING_SEGMENT {
        SoukKind::Sharings
    } else if col.value().classes().any(|c| c == sel::SOUK_CARD_REQUEST_CLASS) || first(col, sel::SOUK_CARD_REQUEST_BADGE).is_some() {
        SoukKind::Requests
    } else if col.value().classes().any(|c| c == sel::SOUK_CARD_OFFER_CLASS) || first(col, sel::SOUK_CARD_OFFER_BADGE).is_some() {
        SoukKind::Offers
    } else {
        return None;
    };
    let name = first(col, sel::SOUK_CARD_NAME)?;
    let mut item = SoukItem {
        id: ids[3].parse().ok()?,
        kind,
        url: url.to_string(),
        perfume_name: none_if_empty(own_text(name))?,
        concentration: first(col, sel::SOUK_CARD_CONCENTRATION).map(text_of).and_then(none_if_empty),
        brand: first(col, sel::SOUK_CARD_BRAND).map(text_of).and_then(strip_dash),
        author: ids[1].to_string(),
        author_url: url[..url.find("/Souk/")?].to_string(),
        country_code: flag_code(col, sel::SOUK_CARD_FLAG),
        date: first(col, sel::SOUK_CARD_DATE).map(text_of).and_then(none_if_empty),
        has_photos: first(col, sel::SOUK_CARD_PHOTO).is_some(),
        ..SoukItem::default()
    };
    for label in col.select(&s(sel::SOUK_CARD_LABEL)) {
        let text = text_of(label);
        if first(label, sel::SOUK_CARD_LABEL_LEFT).is_some() {
            item.size = none_if_empty(text);
        } else if first(label, sel::SOUK_CARD_LABEL_PEOPLE).is_some() {
            item.participants = parse_int(&text);
        } else if item.kind == SoukKind::Sharings {
            // A split without amount left carries a plain status label (`Complete`).
            item.status.extend(none_if_empty(text));
        } else {
            item.item_type = none_if_empty(text);
        }
    }
    Some(item)
}

fn souk_cards(root: ElementRef) -> PResult<Vec<SoukItem>> {
    root.select(&s(sel::SOUK_CARD)).map(|col| souk_card(col).ok_or_else(|| outdated("Souk list: entry without link, kind or perfume name"))).collect()
}

/// One page of a site-wide Souk list (`/Souks/Offers`, `/Souks/Requests`, `/Souks/Sharings`:
/// 16 per page) or of a Souk search (`/s_souk.php`: 20 per page).
pub fn parse_souk_listing(html: &str, requested_page: u32) -> PResult<Listing<SoukItem>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::SOUK_SEARCH_FORM).is_none() {
        return Err(outdated("Souk list: search form not found"));
    }
    let mut items = souk_cards(root)?;
    if items.is_empty() {
        if first(root, sel::NOTHING_FOUND).is_none() {
            return Err(outdated("Souk list: neither an entry nor the 'nothing found' marker"));
        }
        // On page 1 this means "no result"; further pages only tell that the page is out of range.
        let total = if requested_page == 1 { Some(0) } else { None };
        return Ok(Listing { items, page: requested_page, total_pages: total, total_items: total });
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

/// One tab of `/Users/{nick}/Souk`: its entries, the paths of the other tabs that hold
/// entries (offers / requests × bottles / samples, splits) and the number of pages of this tab.
pub fn parse_user_souk(html: &str) -> PResult<(Vec<SoukItem>, Vec<String>, u32)> {
    let doc = Html::parse_document(html);
    let root = user_text_page(&doc, "Souk")?;
    let tabs: Vec<ElementRef> = root.select(&s(sel::USER_SOUK_TABS)).collect();
    if tabs.is_empty() {
        return Err(outdated("member Souk page: tabs not found"));
    }
    let items = souk_cards(root)?;
    if items.is_empty() && first(root, sel::NOTHING_FOUND).is_none() {
        return Err(outdated("member Souk page: neither an entry nor the 'nothing yet' marker"));
    }
    let mut others = Vec::new();
    for tab in tabs {
        let Some(href) = tab.value().attr("href") else { continue };
        let filled = first(tab, sel::TAB_COUNT).and_then(|c| parse_int(&text_of(c))).is_some_and(|n| n > 0);
        let path = routes::user_souk_tab(url_path(href));
        if filled && !path.contains(routes::USER_SOUK_CLOSED) && !others.contains(&path) {
            others.push(path);
        }
    }
    let (_, max_link) = paging_marks(root);
    Ok((items, others, max_link.unwrap_or(1)))
}

/// A Souk entry page (`/Users/{nick}/Souk/Item/{id}` or `/Souk/Sharing/{id}`). `url` is the
/// absolute URL it was read from (the page has no canonical link).
pub fn parse_souk_item(html: &str, url: &str) -> PResult<SoukItem> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let Some(head) = first(root, sel::SOUK_HEAD) else { return Err(outdated("Souk entry: head block not found")) };
    let ids = rx(re::SOUK_ITEM_URL).captures(url).ok_or_else(|| outdated("Souk entry: unexpected entry URL"))?;
    let perfume = first(head, sel::SOUK_HEAD_PERFUME);
    let headline = first(head, sel::SOUK_HEAD_TYPE).map(text_of);
    let id = ids[3].parse::<u64>().ok();
    let (Some(perfume), Some(headline), Some(id)) = (perfume, headline, id) else {
        return Err(outdated("Souk entry: perfume, headline or id missing"));
    };
    let (kind, item_type) = if &ids[2] == markers::SOUK_SHARING_SEGMENT {
        (SoukKind::Sharings, None)
    } else if let Some(rest) = headline.strip_prefix(markers::SOUK_HEAD_REQUEST) {
        (SoukKind::Requests, none_if_empty(squash(rest)))
    } else if let Some(rest) = headline.strip_prefix(markers::SOUK_HEAD_OFFER) {
        (SoukKind::Offers, none_if_empty(squash(rest)))
    } else {
        return Err(outdated("Souk entry: headline is neither an offer nor a request"));
    };
    let perfume_url = perfume.value().attr("href").unwrap_or_default();
    let member = first(head, sel::SOUK_HEAD_MEMBER);
    let mut item = SoukItem {
        id,
        kind,
        url: url.to_string(),
        perfume_name: text_of(perfume),
        brand: first(head, sel::SOUK_HEAD_BRAND).map(text_of).and_then(none_if_empty),
        perfume_id: perfume_id_from_url(perfume_url),
        perfume_url: none_if_empty(perfume_url.to_string()),
        author: ids[1].to_string(),
        author_url: member
            .and_then(|a| a.value().attr("href"))
            .map(|href| href.trim_end_matches("/Souk"))
            .unwrap_or(&url[..url.find("/Souk/").unwrap_or(url.len())])
            .to_string(),
        author_id: rx(re::SOUK_MEMBER_ID).captures(html).and_then(|c| c[1].parse().ok()),
        country: first(root, sel::SOUK_COUNTRY).and_then(|i| i.parent()).and_then(ElementRef::wrap).map(text_of).and_then(none_if_empty),
        country_code: flag_code(root, sel::SOUK_COUNTRY),
        item_type,
        ..SoukItem::default()
    };
    for (i, line) in head.select(&s(sel::SOUK_HEAD_META)).enumerate() {
        // First line: `<a>nick</a> 19 hours ago`; the next ones are status lines of a split.
        if i == 0 {
            item.date = none_if_empty(own_text(line));
        } else if let Some(text) = none_if_empty(text_of(line)) {
            item.status.push(text);
        }
    }
    if let Some(second) = first(head, sel::SOUK_HEAD_SECOND) {
        if first(second, sel::SOUK_CARD_LABEL_PEOPLE).is_some() {
            item.participants = parse_int(&text_of(second));
        } else {
            item.preference = none_if_empty(text_of(second));
        }
    }
    for row in root.select(&s(sel::SOUK_ROW)) {
        let cells: Vec<ElementRef> = row.select(&s(sel::SOUK_ROW_CELL)).collect();
        match cells.as_slice() {
            [note] => item.note = none_if_empty(rich_text(*note, None)),
            [label, value] => {
                let (label, value) = (text_of(*label), text_of(*value));
                if value.is_empty() {
                    continue;
                }
                let lower = label.to_lowercase();
                let slot = match markers::SOUK_ROWS.iter().find(|(needle, _)| lower.contains(needle)).map(|(_, field)| *field) {
                    Some("trade") => &mut item.trade,
                    Some("condition") => &mut item.condition,
                    Some("price") => &mut item.price,
                    Some("shipping") => &mut item.shipping,
                    Some("swap_for") => &mut item.swap_for,
                    _ => {
                        item.details.push(SoukDetail { label, value });
                        continue;
                    }
                };
                *slot = Some(value);
            }
            _ => {}
        }
    }
    let ring: Vec<String> =
        [sel::SOUK_RING_VALUE, sel::SOUK_RING_LABEL].iter().filter_map(|css| first(root, css).map(text_of)).filter(|t| !t.is_empty()).collect();
    item.size = none_if_empty(ring.join(" "));
    item.size_label = first(root, sel::SOUK_RING_CAPTION).map(text_of).and_then(none_if_empty);
    item.photos = root.select(&s(sel::SOUK_PHOTO)).filter_map(|a| a.value().attr("href")).map(str::to_string).collect();
    item.has_photos = !item.photos.is_empty();
    Ok(item)
}

// ---------------------------------------------------------------------------
// Account: ShelfControl (own collection) and wear tracker — jalon 2
// ---------------------------------------------------------------------------

/// `YYYY-MM-DD` (UTC) of a Unix timestamp.
pub fn iso_date(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Proleptic Gregorian date of a day count since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn json_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().replace(',', ".").parse().ok()))
}

fn json_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => none_if_empty(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Name stored under `id` in one of the ShelfControl dictionaries. Entries are either a
/// string or `[name, url-or-html]`; an empty dictionary is serialised as `[]`.
fn shelf_dict(data: &serde_json::Value, dict: &str, id: &serde_json::Value) -> Option<String> {
    let key = json_text(id)?;
    match data.get(dict)?.get(&key)? {
        serde_json::Value::Array(a) => a.first().and_then(json_text),
        other => json_text(other),
    }
}

fn shelf_names(data: &serde_json::Value, dict: &str, ids: &serde_json::Value) -> Vec<String> {
    ids.as_array().map(|a| a.iter().filter_map(|id| shelf_dict(data, dict, id)).collect()).unwrap_or_default()
}

fn shelf_list(v: &serde_json::Value) -> Option<CollectionList> {
    let id = json_u64(v.get("id")?)?;
    Some(CollectionList {
        id,
        slug: json_text(v.get("slug")?)?,
        name: json_text(v.get("name")?).unwrap_or_default(),
        notes: v.get("notes").and_then(json_text),
        hidden: v.get("hidden").and_then(|h| h.as_str()).is_some_and(|h| h.eq_ignore_ascii_case("yes")),
        count: v.get("count").and_then(json_u64).unwrap_or(0) as u32,
        custom: id > routes::LAST_FIXED_LIST_ID,
        url: v.get("url").and_then(json_text).unwrap_or_default(),
    })
}

/// Member rating of the ShelfControl JSON → 0-10 scale (the write API uses 0-100).
fn shelf_rating(v: Option<&serde_json::Value>) -> Option<f32> {
    let raw = json_f64(v?)? as f32;
    Some(if raw > 10.0 { raw / 10.0 } else { raw })
}

fn shelf_item(data: &serde_json::Value, row: &serde_json::Value, columns: &[CustomColumn]) -> Option<CollectionItem> {
    let url = json_text(row.get("url")?)?;
    let ratings = row.get("ratings").and_then(|r| r.as_array());
    let rating = |i: usize| shelf_rating(ratings.and_then(|r| r.get(i)));
    let ts = |key: &str| row.get(key).and_then(|v| v.as_i64()).filter(|t| *t > 0).map(iso_date);
    let positive = |key: &str| row.get(key).and_then(json_f64).filter(|v| *v > 0.0);
    let custom = row
        .get("custom")
        .and_then(|c| c.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(id, value)| {
                    let value = match value {
                        serde_json::Value::Bool(b) => Some(b.to_string()),
                        other => json_text(other),
                    }?;
                    let name = columns.iter().find(|c| c.id.to_string() == *id).map(|c| c.name.clone()).unwrap_or_else(|| format!("column {id}"));
                    Some((name, value))
                })
                .collect()
        })
        .unwrap_or_default();
    let gender = row
        .get("gender")
        .and_then(|g| g.as_str())
        .and_then(|g| markers::SHELF_GENDERS.iter().find(|(code, _)| *code == g).map(|(_, v)| v.to_string()));
    Some(CollectionItem {
        entry_id: json_u64(row.get("id")?)?,
        perfume_id: perfume_id_from_url(&url)?,
        p_id: json_u64(row.get("p")?)?,
        name: row.get("title").and_then(json_text)?,
        brand: row.get("brand").and_then(|b| shelf_dict(data, "brands", b)),
        url,
        year: row.get("year").and_then(json_u64).filter(|y| *y > 0).map(|y| y as u16),
        gender,
        concentration: row.get("conc").and_then(json_text),
        my_ratings: MemberRatings { scent: rating(0), durability: rating(1), sillage: rating(2), bottle: rating(3), pricing: rating(4) },
        community_rating: row.get("community").and_then(json_f64).filter(|v| *v > 0.0).map(|v| v as f32),
        community_votes: row.get("votes").and_then(json_u64).map(|v| v as u32),
        quantity: row.get("qty").and_then(json_u64).map(|v| v as u32),
        bottle_size: row.get("bottle").and_then(json_text).filter(|b| b != "0"),
        remaining_ml: positive("ml"),
        price_paid: positive("paid"),
        currency: row.get("cur").and_then(json_text),
        private_note: row.get("memo").and_then(json_text),
        added: ts("added"),
        wear_count: row.get("worn").and_then(json_u64).unwrap_or(0) as u32,
        last_worn: ts("last"),
        custom,
        has_statement: row.get("statement").and_then(json_u64).unwrap_or(0) > 0,
        has_review: row.get("review").and_then(json_u64).unwrap_or(0) > 0,
        perfumers: row.get("perfumers").map(|v| shelf_names(data, "perfumers", v)).unwrap_or_default(),
        accords: row.get("accords").map(|v| shelf_names(data, "accords", v)).unwrap_or_default(),
        notes: row.get("notes").map(|v| shelf_names(data, "notes", v)).unwrap_or_default(),
        seasons: row.get("seasons").map(|v| shelf_names(data, "classes", v)).unwrap_or_default(),
        occasions: row.get("occasions").map(|v| shelf_names(data, "classes", v)).unwrap_or_default(),
        country: row.get("country").and_then(|c| shelf_dict(data, "countries", c)),
    })
}

/// A ShelfControl page (`/account/shelfcontrol/{list}`): the member's lists, the custom
/// columns and every row of the active list (DISCOVERY §5).
pub fn parse_shelf(html: &str) -> PResult<Shelf> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let wrap = first(root, sel::SHELF_WRAP).ok_or_else(|| outdated("ShelfControl: list container not found"))?;
    let lists_json: serde_json::Value = wrap
        .value()
        .attr("data-lists")
        .and_then(|raw| serde_json::from_str(raw).ok())
        .ok_or_else(|| outdated("ShelfControl: data-lists is not JSON"))?;
    let lists: Vec<CollectionList> = lists_json.as_array().map(|a| a.iter().filter_map(shelf_list).collect()).unwrap_or_default();
    if lists.is_empty() {
        return Err(outdated("ShelfControl: no list in data-lists"));
    }
    let active: u64 =
        wrap.value().attr("data-list").and_then(|v| v.trim().parse().ok()).ok_or_else(|| outdated("ShelfControl: data-list missing"))?;
    let list = lists.iter().find(|l| l.id == active).cloned().ok_or_else(|| outdated("ShelfControl: active list not in data-lists"))?;

    let col_id = rx(re::SHELF_CUSTOM_COL_ID);
    let columns: Vec<CustomColumn> = root
        .select(&s(sel::SHELF_CUSTOM_COL))
        .filter_map(|th| {
            let id = col_id.captures(th.value().attr("data-col")?)?[1].parse().ok()?;
            Some(CustomColumn { id, name: text_of(th) })
        })
        .collect();

    let script = first(root, sel::SHELF_DATA).ok_or_else(|| outdated("ShelfControl: embedded JSON not found"))?;
    let data: serde_json::Value =
        serde_json::from_str(script.text().collect::<String>().trim()).map_err(|_| outdated("ShelfControl: embedded JSON is invalid"))?;
    let rows = data.get("rows").and_then(|r| r.as_array()).ok_or_else(|| outdated("ShelfControl: no rows array"))?;
    let items: Vec<CollectionItem> = rows.iter().filter_map(|row| shelf_item(&data, row, &columns)).collect();
    if items.len() != rows.len() {
        return Err(outdated("ShelfControl: a row lacks id, perfume id, title or URL"));
    }
    Ok(Shelf { list, lists, columns, items })
}

/// `Thursday, 12/11/2026, 07:21 AM` → `2026-11-12T07:21`.
pub fn parse_wear_when(text: &str) -> Option<String> {
    let c = rx(re::WEAR_WHEN).captures(text)?;
    let n = |i: usize| c[i].parse::<u32>().ok();
    let (day, month, year, mut hour, minute) = (n(1)?, n(2)?, n(3)?, n(4)?, n(5)?);
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    hour %= 12;
    if &c[6] == "PM" {
        hour += 12;
    }
    Some(format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}"))
}

fn wear_rows(root: ElementRef) -> PResult<Vec<Wear>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in root.select(&s(sel::WEAR_ROW)) {
        let id = row.value().attr("id").and_then(|id| rx(re::TRAILING_ID).captures(id).and_then(|c| c[1].parse::<u64>().ok()));
        let link = attr(row, sel::WEAR_LINK, "href");
        let (Some(wear_id), Some(url)) = (id, link) else { return Err(outdated("wear tracker: row without id or perfume link")) };
        if !seen.insert(wear_id) {
            continue;
        }
        let (alt_name, brand) = attr(row, sel::WEAR_IMG, "alt").map(split_alt).unwrap_or_default();
        let name = if alt_name.is_empty() { first(row, sel::WEAR_LINK).map(text_of).unwrap_or_default() } else { alt_name };
        let when = first(row, sel::WEAR_WHEN);
        out.push(Wear {
            wear_id,
            perfume_id: perfume_id_from_url(url).ok_or_else(|| outdated("wear tracker: unexpected perfume link"))?,
            name,
            brand,
            url: url.to_string(),
            worn_at: when.and_then(|w| parse_wear_when(&text_of(w))),
            comment: when.and_then(|w| attr(w, sel::WEAR_NOTE_ICON, "title")).map(squash).and_then(none_if_empty),
        });
    }
    Ok(out)
}

/// Wears of one month, from the `calendar_month` fragment. An empty month is `Ok(vec![])`.
pub fn parse_wear_calendar(fragment: &str) -> PResult<Vec<Wear>> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    if first(root, sel::WEAR_CALENDAR).is_none() {
        return Err(outdated("wear calendar: month grid not found"));
    }
    wear_rows(root)
}

/// Wears listed on the tracker overview page (`/assistant/tracker`), most recent first.
pub fn parse_wear_tracker(html: &str) -> PResult<Vec<Wear>> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::WEAR_TRACKER_TABS).is_none() {
        return Err(outdated("wear tracker: page tabs not found"));
    }
    wear_rows(root)
}

// ---------------------------------------------------------------------------
// Account: ratings, classification, reviews and statements — jalon 3
// ---------------------------------------------------------------------------

/// Nickname of the logged-in member, from the header of any page served to a session.
pub fn parse_own_nick(html: &str) -> PResult<String> {
    let doc = Html::parse_document(html);
    first(doc.root_element(), sel::NICK_NAME)
        .map(own_text)
        .filter(|n| !n.is_empty() && !n.contains(['/', '?', '#', ' ']))
        .ok_or_else(|| outdated("page header: nickname of the logged-in member not found"))
}

/// The member's five ratings of a perfume, out of 10, from the `perfume_rate_panel.php`
/// fragment. A scale the member did not rate is `None`: its remove link is hidden.
pub fn parse_rate_panel(fragment: &str) -> PResult<MemberRatings> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let scale_re = rx(re::RATE_SCALE_CLASS);
    let remove_re = rx(re::RATE_REMOVE_CLASS);
    let mut removable: Vec<(String, bool)> = Vec::new();
    for link in root.select(&s(sel::RATE_REMOVE)) {
        let class = link.value().attr("class").unwrap_or_default();
        if let Some(c) = remove_re.captures(class) {
            removable.push((c[1].to_string(), !link.value().classes().any(|k| k == "hide")));
        }
    }
    let mut out = MemberRatings::default();
    let mut seen = HashSet::new();
    for stars in root.select(&s(sel::RATE_STARS)) {
        let class = stars.value().attr("class").unwrap_or_default();
        let Some(scale) = scale_re.captures(class).map(|c| c[1].to_string()) else { continue };
        let value: f32 = stars
            .value()
            .attr("data-pstars-value")
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| outdated("rating panel: star value unreadable"))?;
        if !(0.0..=5.0).contains(&value) {
            return Err(outdated("rating panel: star value out of range"));
        }
        let rated = removable.iter().find(|(sc, _)| *sc == scale).map(|(_, shown)| *shown).unwrap_or(value > 0.0);
        out.set(&scale, rated.then_some(value * 2.0));
        seen.insert(scale);
    }
    if seen.len() != routes::RATING_SCALES.len() {
        return Err(outdated("rating panel: the five rating scales were not found"));
    }
    Ok(out)
}

/// Every label of the `perfume_classify_panel.php` fragment, with the member's votes.
pub fn parse_classify_panel(fragment: &str) -> PResult<Vec<ClassChoice>> {
    let doc = Html::parse_fragment(fragment);
    let mut out = Vec::new();
    for group in doc.root_element().select(&s(sel::CLASSI_GROUP)) {
        let name = first(group, sel::CLASSI_GROUP_TITLE)
            .map(text_of)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| outdated("classification panel: group title missing"))?;
        for option in group.select(&s(sel::CLASSI_OPTION)) {
            let id = |a: &str| option.value().attr(a).and_then(|v| v.parse::<u64>().ok());
            let (Some(group_id), Some(label_id)) = (id("data-c"), id("data-ct")) else {
                return Err(outdated("classification panel: option without ids"));
            };
            let label = text_of(option);
            if label.is_empty() {
                return Err(outdated("classification panel: option without label"));
            }
            out.push(ClassChoice {
                group: name.clone(),
                label,
                voted: option.value().classes().any(|k| k == sel::CLASSI_VOTED_CLASS),
                group_id,
                label_id,
            });
        }
    }
    if out.is_empty() {
        return Err(outdated("classification panel: no label found"));
    }
    Ok(out)
}

/// The member's own review or statement on a perfume (`get_user_review`,
/// `get_user_statement`): empty strings when there is none.
pub fn parse_own_draft(body: &str, kind: TextKind) -> PResult<OwnDraft> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).map_err(|_| outdated("own review / statement: answer is not JSON"))?;
    let field = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|t| t.trim().to_string());
    match kind {
        TextKind::Review => {
            let text = field("review_text").ok_or_else(|| outdated("own review: `review_text` missing"))?;
            Ok(OwnDraft { title: field("review_title").and_then(none_if_empty), text })
        }
        TextKind::Statement => {
            Ok(OwnDraft { title: None, text: field("statement_text").ok_or_else(|| outdated("own statement: `statement_text` missing"))? })
        }
    }
}

/// Delete button of the review or statement form (`data-id`, `data-p`, `data-h`). `None`
/// when the form shows no such button, i.e. the member has nothing to delete there.
pub fn parse_delete_handle(fragment: &str, kind: TextKind) -> PResult<Option<DeleteHandle>> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let form_p = attr(root, sel::FORM_PERFUME_ID, "value")
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| outdated("review / statement form: perfume id missing"))?;
    let css = match kind {
        TextKind::Review => sel::REVIEW_DELETE_BTN,
        TextKind::Statement => sel::STATEMENT_DELETE_BTN,
    };
    let Some(button) = first(root, css) else { return Ok(None) };
    let data = |a: &str| button.value().attr(a).map(str::trim);
    let id = data("data-id").and_then(|v| v.parse::<u64>().ok()).filter(|id| *id > 0);
    let h = data("data-h").filter(|h| rx(re::H_TOKEN).is_match(h));
    let (Some(id), Some(h)) = (id, h) else { return Err(outdated("review / statement form: delete button without id or token")) };
    Ok(Some(DeleteHandle { id, p_id: data("data-p").and_then(|v| v.parse().ok()).unwrap_or(form_p), h: h.to_string() }))
}

/// One page of the member's "My Ratings" list for one scale.
pub fn parse_my_ratings(html: &str, requested_page: u32) -> PResult<Listing<RatedPerfume>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::RATINGS_TABS).is_none() {
        return Err(outdated("ratings page: scale tabs not found"));
    }
    let trailing = rx(re::TRAILING_ID);
    let mut items = Vec::new();
    for row in root.select(&s(sel::RATINGS_ITEM)) {
        let p_id = row.value().attr("id").and_then(|i| trailing.captures(i)).and_then(|c| c[1].parse::<u64>().ok());
        let link = first(row, sel::RATINGS_LINK);
        let stars = attr(row, sel::RATE_STARS, "data-pstars-value").and_then(|v| v.trim().parse::<f32>().ok());
        let (Some(p_id), Some(link), Some(stars)) = (p_id, link, stars) else {
            return Err(outdated("ratings page: item without perfume id, link or rating"));
        };
        let url = link.value().attr("href").unwrap_or_default().to_string();
        let (alt_name, brand) = attr(row, sel::RATINGS_IMG, "alt").map(split_alt).unwrap_or_default();
        items.push(RatedPerfume {
            perfume_id: perfume_id_from_url(&url).ok_or_else(|| outdated("ratings page: unexpected perfume link"))?,
            p_id,
            name: if alt_name.is_empty() { text_of(link) } else { alt_name },
            brand,
            url,
            rating: stars * 2.0,
            rated: first(row, sel::RATINGS_AGE).map(text_of).and_then(none_if_empty),
        });
    }
    if items.is_empty() && first(root, sel::NOTHING_FOUND).is_none() && requested_page == 1 {
        return Err(outdated("ratings page: neither a rating nor the 'no ratings' marker"));
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

fn user_text_page<'a>(doc: &'a Html, what: &str) -> PResult<ElementRef<'a>> {
    let root = doc.root_element();
    if first(root, sel::USER_PAGE_HEAD).is_none() {
        return Err(outdated(&format!("member {what} page: heading not found")));
    }
    Ok(root)
}

fn finish_user_texts(root: ElementRef, requested_page: u32, mut items: Vec<OwnText>, what: &str) -> PResult<Listing<OwnText>> {
    if items.is_empty() && first(root, sel::NOTHING_FOUND).is_none() && requested_page == 1 {
        return Err(outdated(&format!("member {what} page: neither an item nor the 'nothing yet' marker")));
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

/// One page of `/Users/{nick}/reviews` (5 reviews per page, full text).
pub fn parse_user_reviews(html: &str, requested_page: u32) -> PResult<Listing<OwnText>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = user_text_page(&doc, "reviews")?;
    let class_id = rx(re::USER_REVIEW_CLASS_ID);
    let mut items = Vec::new();
    for block in root.select(&s(sel::USER_REVIEW)) {
        let id = block.value().attr("class").and_then(|c| class_id.captures(c)).and_then(|c| c[1].parse::<u64>().ok());
        let perfume = first(block, sel::USER_TEXT_PERFUME);
        let text_el = first(block, sel::REVIEW_TEXT);
        let (Some(id), Some(perfume), Some(text_el)) = (id, perfume, text_el) else {
            return Err(outdated("member reviews page: review without id, perfume or text"));
        };
        let perfume_url = perfume.value().attr("href").unwrap_or_default();
        items.push(OwnText {
            id,
            url: attr(block, sel::USER_REVIEW_LINK, "href")
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}/reviews/{id}", perfume_url.trim_end_matches('/'))),
            perfume_id: perfume_id_from_url(perfume_url).ok_or_else(|| outdated("member reviews page: unexpected perfume link"))?,
            perfume_name: text_of(perfume),
            brand: first(block, sel::USER_TEXT_BRAND).map(text_of).and_then(strip_dash),
            date: first(block, sel::USER_REVIEW_AGE).map(text_of).and_then(none_if_empty),
            title: first(block, sel::REVIEW_TITLE).map(text_of).and_then(none_if_empty),
            text: rich_text(text_el, None),
            ratings: member_ratings(block),
            awards: first(block, sel::USER_REVIEW_AWARDS).and_then(|e| parse_int(&text_of(e))),
            comments: first(block, sel::REVIEW_COMMENTS).and_then(|e| parse_int(&text_of(e))),
        });
    }
    finish_user_texts(root, requested_page, items, "reviews")
}

/// One page of `/Users/{nick}/statements`.
pub fn parse_user_statements(html: &str, requested_page: u32) -> PResult<Listing<OwnText>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = user_text_page(&doc, "statements")?;
    let trailing = rx(re::TRAILING_ID);
    let mut items = Vec::new();
    for block in root.select(&s(sel::STATEMENT)) {
        let id = block.value().attr("id").and_then(|i| trailing.captures(i)).and_then(|c| c[1].parse::<u64>().ok());
        let perfume = first(block, sel::USER_STATEMENT_PERFUME);
        let text_el = first(block, sel::STATEMENT_TEXT);
        let (Some(id), Some(perfume), Some(text_el)) = (id, perfume, text_el) else {
            return Err(outdated("member statements page: statement without id, perfume or text"));
        };
        let perfume_url = perfume.value().attr("href").unwrap_or_default();
        items.push(OwnText {
            id,
            url: format!("{}/statements/{id}", perfume_url.trim_end_matches('/')),
            perfume_id: perfume_id_from_url(perfume_url).ok_or_else(|| outdated("member statements page: unexpected perfume link"))?,
            perfume_name: text_of(perfume),
            brand: first(block, sel::USER_TEXT_BRAND).map(text_of).and_then(none_if_empty),
            date: None,
            title: None,
            text: rich_text(text_el, None),
            ratings: member_ratings(block),
            awards: first(block, sel::STATEMENT_AWARDS).and_then(|e| parse_int(&text_of(e))),
            comments: first(block, sel::STATEMENT_COMMENTS).and_then(|e| parse_int(&text_of(e))),
        });
    }
    finish_user_texts(root, requested_page, items, "statements")
}

// ---------------------------------------------------------------------------
// Forum and communities (jalon 4, read only)
// ---------------------------------------------------------------------------

/// The footer closes every page of the site: without it the body was cut short, and the
/// items found so far must not be taken for a complete (shorter) page.
fn require_footer(root: ElementRef, what: &str) -> PResult<()> {
    match first(root, sel::PAGE_FOOTER) {
        Some(_) => Ok(()),
        None => Err(outdated(&format!("{what}: page footer not found (incomplete page)"))),
    }
}

/// `https://host` of an absolute URL (empty for a path).
fn origin_of(url: &str) -> &str {
    match url.find("://") {
        Some(i) => match url[i + 3..].find('/') {
            Some(j) => &url[..i + 3 + j],
            None => url,
        },
        None => "",
    }
}

/// First capture of `pattern` in `text`, read as a count (`16,488` → 16488).
fn count_in(text: &str, pattern: &str) -> Option<u32> {
    let caps = rx(pattern).captures(text)?;
    caps[1].chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// Highest and active page numbers of a pagination block whose links are not `current_page=` links.
fn numbered_paging(root: ElementRef) -> (Option<u32>, Option<u32>) {
    let number = |e: ElementRef| text_of(e).parse::<u32>().ok();
    (root.select(&s(sel::PAGING_NUMBERS)).filter_map(number).max(), first(root, sel::PAGING_ACTIVE).and_then(number))
}

/// Listing of a forum page: the page is the one the site marks active, the totals come
/// from the "1 - 25 by 62" counter and the page numbers.
fn forum_listing<T>(root: ElementRef, requested_page: u32, per_page: u32, items: Vec<T>) -> Listing<T> {
    let (range, _) = paging_marks(root);
    let (max_page, active) = numbered_paging(root);
    let total_items = match (range, max_page) {
        (Some((_, _, total)), _) => Some(total),
        // No pagination block at all: a single page.
        (None, None) => Some(items.len() as u32),
        (None, Some(_)) => None,
    };
    let total_pages = max_page.or(total_items.map(|t| t.div_ceil(per_page.max(1)).max(1)));
    Listing { items, page: active.unwrap_or(requested_page.max(1)), total_pages, total_items }
}

/// The `forum_context` JSON of category and topic pages.
fn forum_context(root: ElementRef) -> Option<serde_json::Value> {
    let raw: String = first(root, sel::FORUM_CONTEXT)?.text().collect();
    serde_json::from_str(raw.trim()).ok()
}

fn user_link(scope: ElementRef, css: &str) -> (Option<String>, Option<String>) {
    match first(scope, css) {
        Some(a) => (none_if_empty(text_of(a)), a.value().attr("href").map(str::to_string)),
        None => (None, None),
    }
}

/// Categories of the forum home (`/forums`), in site order, with their section.
pub fn parse_forum_home(html: &str) -> PResult<Vec<ForumCategory>> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::FORUM_HOME).is_none() {
        return Err(outdated("forum home: category overview not found"));
    }
    require_footer(root, "forum home")?;
    let icon_id = rx(re::FORUM_ICON_ID);
    let mut section: Option<String> = None;
    let mut out = Vec::new();
    for el in root.select(&s(sel::FORUM_HOME_ENTRIES)) {
        if !el.value().classes().any(|c| c == sel::FORUM_ROW_CLASS) {
            section = none_if_empty(text_of(el));
            continue;
        }
        let link = first(el, sel::FORUM_ROW_LINK);
        let href = link.and_then(|a| a.value().attr("href")).unwrap_or_default();
        let (Some(link), Some(slug)) = (link, slug_after(href, routes::FORUM_PREFIX)) else {
            return Err(outdated("forum home: category row without link"));
        };
        let stats = first(el, sel::FORUM_ROW_STATS).map(text_of).unwrap_or_default();
        out.push(ForumCategory {
            forum_id: attr(el, sel::FORUM_ROW_ICON, "src").and_then(|src| icon_id.captures(src)).and_then(|c| c[1].parse().ok()),
            name: text_of(link),
            description: first(el, sel::FORUM_ROW_DESC).map(text_of).and_then(none_if_empty),
            section: section.clone(),
            topics: count_in(&stats, re::COUNT_TOPICS),
            topics_today: count_in(&stats, re::COUNT_TODAY),
            url: format!("{}{}", origin_of(href), routes::forum_category(&slug, 1)),
            slug,
        });
    }
    if out.is_empty() {
        return Err(outdated("forum home: no category row"));
    }
    Ok(out)
}

fn topic_row(row: ElementRef, pinned: bool) -> Option<ForumTopic> {
    let link = first(row, sel::FORUM_ROW_LINK)?;
    let href = link.value().attr("href")?;
    let slug = slug_after(href, routes::FORUM_TOPIC_PREFIX)?;
    let (author, author_url) = user_link(row, sel::TOPIC_AUTHOR);
    let last = first(row, sel::TOPIC_LAST_POST);
    let last_id = last.and_then(|a| a.value().attr("href")).and_then(|h| rx(re::FORUM_POST_ID).captures(h)).and_then(|c| c[1].parse::<u64>().ok());
    Some(ForumTopic {
        title: text_of(link),
        url: format!("{}{}", origin_of(href), routes::forum_topic(&slug, 1)),
        author,
        author_url,
        replies: first(row, sel::TOPIC_META).and_then(|m| count_in(&text_of(m), re::TOPIC_REPLIES)),
        pages: attr(row, sel::TOPIC_LAST_PAGE, "href").and_then(|h| count_in(url_path(h), re::TRAILING_ID)),
        last_post_id: last_id,
        last_post_url: last_id.map(|id| format!("{}{}#{id}", origin_of(href), routes::forum_post(id))),
        last_post_author: first(row, sel::TOPIC_LAST_AUTHOR).map(text_of).and_then(none_if_empty),
        last_post_date: last.map(text_of).and_then(none_if_empty),
        pinned,
        locked: first(row, sel::TOPIC_LOCKED).is_some(),
        slug,
    })
}

/// One page of a forum category (`/forums/{slug}` then `/forums/{slug}/{page}`, 50 topics).
pub fn parse_forum_category(html: &str, requested_page: u32) -> PResult<ForumTopicsPage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("forum category".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    require_footer(root, "forum category page")?;
    let Some(ctx) = forum_context(root) else { return Err(outdated("forum category page: forum_context not found")) };
    let Some(h1) = first(root, sel::FORUM_CATEGORY_H1) else { return Err(outdated("forum category page: heading not found")) };
    let canonical = attr(root, sel::CANONICAL, "href").unwrap_or_default();
    let slug = slug_after(canonical, routes::FORUM_PREFIX)
        .filter(|slug| !routes::FORUM_RESERVED.contains(&slug.as_str()))
        .ok_or_else(|| outdated("forum category page: canonical link is not a category"))?;
    let pinned: HashSet<_> = root.select(&s(sel::TOPIC_PINNED_ROW)).map(|e| e.id()).collect();
    let mut items = Vec::new();
    for row in root.select(&s(sel::TOPIC_ROW)) {
        items.push(topic_row(row, pinned.contains(&row.id())).ok_or_else(|| outdated("forum category page: topic row without topic link"))?);
    }
    let topics = forum_listing(root, requested_page, routes::FORUM_TOPICS_PER_PAGE, items);
    let category = ForumCategory {
        forum_id: ctx.get(markers::FORUM_CONTEXT_FORUM_ID).and_then(json_u64),
        name: own_text(h1),
        description: first(root, sel::FORUM_CATEGORY_DESC).map(text_of).and_then(none_if_empty),
        section: None,
        topics: topics.total_items,
        topics_today: None,
        url: format!("{}{}", origin_of(canonical), routes::forum_category(&slug, 1)),
        slug,
    };
    Ok(ForumTopicsPage { category, topics })
}

/// One page of a forum topic (`/forums/topic/{slug}[/{page}]` or `/forums/post/{id}`, 25 posts).
pub fn parse_forum_thread(html: &str, requested_page: u32) -> PResult<ForumThread> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("forum topic".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    require_footer(root, "forum topic page")?;
    let Some(ctx) = forum_context(root) else { return Err(outdated("forum topic page: forum_context not found")) };
    let topic_id = ctx
        .get(markers::FORUM_CONTEXT_TOPIC_ID)
        .and_then(json_u64)
        .ok_or_else(|| outdated("forum topic page: topic_id not found in forum_context"))?;
    let Some(holder) = first(root, sel::THREAD_POSTS) else { return Err(outdated("forum topic page: post list not found")) };
    let Some(h1) = first(root, sel::THREAD_H1) else { return Err(outdated("forum topic page: title not found")) };
    let canonical = attr(root, sel::CANONICAL, "href").unwrap_or_default();
    let slug = slug_after(canonical, routes::FORUM_TOPIC_PREFIX).ok_or_else(|| outdated("forum topic page: canonical link is not a topic"))?;
    let origin = origin_of(canonical);
    let mut items = Vec::new();
    for post in holder.select(&s(sel::POST)) {
        let id = post.value().attr("id").and_then(|i| i.parse::<u64>().ok());
        let author = first(post, sel::POST_AUTHOR).map(text_of).and_then(none_if_empty);
        let text_el = first(post, sel::POST_TEXT);
        let (Some(id), Some(author), Some(text_el)) = (id, author, text_el) else {
            return Err(outdated("forum topic page: post without id, author or text"));
        };
        let date_el = first(post, sel::POST_DATE);
        items.push(ForumPost {
            id,
            url: format!("{origin}{}#{id}", routes::forum_post(id)),
            author,
            author_url: attr(post, sel::POST_AUTHOR_LINK, "href").map(str::to_string),
            date: date_el.map(text_of).and_then(none_if_empty),
            date_iso: date_el.and_then(|d| d.value().attr("content")).map(str::to_string),
            title: first(post, sel::POST_TITLE).map(text_of).and_then(none_if_empty),
            text: rich_text(text_el, None),
            thanks: first(post, sel::POST_THANKS).and_then(|e| parse_int(&text_of(e))),
        });
    }
    if items.is_empty() {
        return Err(outdated("forum topic page: no post"));
    }
    let category = root
        .select(&s(sel::THREAD_CATEGORY))
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let slug = slug_after(href, routes::FORUM_PREFIX).filter(|slug| !routes::FORUM_RESERVED.contains(&slug.as_str()))?;
            Some((slug, text_of(a), href))
        })
        .last();
    let topic = ForumTopicHead {
        topic_id,
        forum_id: ctx.get(markers::FORUM_CONTEXT_FORUM_ID).and_then(json_u64),
        title: text_of(h1),
        url: format!("{origin}{}", routes::forum_topic(&slug, 1)),
        category_slug: category.as_ref().map(|c| c.0.clone()),
        category_name: category.as_ref().and_then(|c| none_if_empty(c.1.clone())),
        category_url: category.as_ref().map(|c| format!("{}{}", origin_of(c.2), routes::forum_category(&c.0, 1))),
        slug,
    };
    Ok(ForumThread { topic, posts: forum_listing(root, requested_page, routes::FORUM_POSTS_PER_PAGE, items) })
}

/// One page of the forum search (`/s_forum_x.php?filter=…`, 20 posts, at most 1000 shown).
pub fn parse_forum_search(html: &str, requested_page: u32) -> PResult<Listing<ForumSearchHit>> {
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    if first(root, sel::FORUM_SEARCH_FORM).is_none() {
        return Err(outdated("forum search page: search form not found"));
    }
    require_footer(root, "forum search page")?;
    let post_id = rx(re::FORUM_POST_ID);
    let mut items = Vec::new();
    for row in root.select(&s(sel::TOPIC_ROW)) {
        let link = first(row, sel::FORUM_ROW_LINK);
        let href = link.and_then(|a| a.value().attr("href")).unwrap_or_default();
        let id = post_id.captures(href).and_then(|c| c[1].parse::<u64>().ok());
        let (Some(link), Some(id)) = (link, id) else { return Err(outdated("forum search page: result without post link")) };
        let (author, author_url) = user_link(row, sel::SEARCH_HIT_AUTHOR);
        let category = first(row, sel::SEARCH_HIT_CATEGORY);
        items.push(ForumSearchHit {
            post_id: id,
            url: format!("{}{}#{id}", origin_of(href), routes::forum_post(id)),
            title: text_of(link),
            author,
            author_url,
            category_slug: category.and_then(|c| c.value().attr("href")).and_then(|h| slug_after(h, routes::FORUM_PREFIX)),
            category_name: category.map(text_of).and_then(none_if_empty),
            date: first(row, sel::TOPIC_META).and_then(|m| own_text(m).rsplit('·').next().map(squash)).and_then(none_if_empty),
            text: first(row, sel::SEARCH_HIT_EXCERPT).map(|e| rich_text(e, None)).and_then(none_if_empty),
        });
    }
    if items.is_empty() {
        if first(root, sel::NOTHING_FOUND).is_none() && requested_page == 1 {
            return Err(outdated("forum search page: neither a result nor the 'nothing found' marker"));
        }
        let total = if requested_page == 1 { Some(0) } else { None };
        return Ok(Listing { items, page: requested_page, total_pages: total, total_items: total });
    }
    let (range, max_link) = paging_marks(root);
    let (total_items, total_pages) = page_bounds(range, max_link, requested_page, &mut items);
    Ok(Listing { items, page: requested_page, total_pages, total_items })
}

/// `(community slug, post slug)` of a community post URL or path.
pub fn community_post_ref(url: &str) -> Option<(String, String)> {
    let rest = url_path(url).strip_prefix(routes::COMMUNITY_PREFIX)?;
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let group = parts.next()?;
    if parts.next()? != routes::COMMUNITY_POST_SEGMENT {
        return None;
    }
    Some((group.to_string(), parts.next()?.to_string()))
}

fn community_slug(url: &str) -> Option<String> {
    slug_after(url, routes::COMMUNITY_PREFIX).filter(|slug| !routes::COMMUNITY_RESERVED.contains(&slug.as_str()))
}

/// Perfumes attached to a community post, each as `Name | Brand`.
fn community_bottles(scope: ElementRef) -> Vec<String> {
    scope
        .select(&s(sel::COMMUNITY_BOTTLE))
        .map(|b| {
            let parts: Vec<String> = b.select(&s(sel::COMMUNITY_BOTTLE_NAME)).map(text_of).filter(|t| !t.is_empty()).collect();
            parts.join(markers::PERFUME_BRAND_SEP)
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn signed_count(text: &str, pattern: &str) -> Option<i32> {
    let caps = rx(pattern).captures(text)?;
    let digits: String = caps[1].chars().filter(|c| c.is_ascii_digit()).collect();
    let value: i32 = digits.parse().ok()?;
    Some(if caps[1].starts_with('-') { -value } else { value })
}

/// One page of the directory of communities (`/communities/discover`, 12 per page).
pub fn parse_communities(html: &str, requested_page: u32) -> PResult<Listing<Community>> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    require_footer(root, "communities directory")?;
    let Some(counter) = first(root, sel::COMMUNITIES_COUNT) else {
        return Err(outdated("communities directory: result counter not found"));
    };
    let mut items = Vec::new();
    for card in root.select(&s(sel::COMMUNITY_CARD)) {
        let href = card.value().attr("href").unwrap_or_default();
        let name = first(card, sel::COMMUNITY_CARD_TITLE).map(text_of).and_then(none_if_empty);
        let (Some(slug), Some(name)) = (community_slug(href), name) else {
            return Err(outdated("communities directory: card without community link or name"));
        };
        let stats: String = card.select(&s(sel::COMMUNITY_CARD_STATS)).map(text_of).collect::<Vec<_>>().join(" · ");
        items.push(Community {
            group_id: None,
            name,
            tagline: first(card, sel::COMMUNITY_CARD_TAGLINE).map(text_of).and_then(none_if_empty),
            description: first(card, sel::COMMUNITY_CARD_DESC).map(text_of).and_then(none_if_empty),
            members: count_in(&stats, re::COUNT_MEMBERS),
            posts: count_in(&stats, re::COUNT_POSTS),
            access: first(card, sel::COMMUNITY_BADGE).map(text_of).and_then(none_if_empty),
            tags: Vec::new(),
            url: format!("{}{}", origin_of(href), routes::community(&slug, 1)),
            slug,
        });
    }
    let (max_page, active) = numbered_paging(root);
    Ok(Listing {
        items,
        page: active.unwrap_or(requested_page.max(1)),
        total_pages: Some(max_page.unwrap_or(1)),
        total_items: count_in(&text_of(counter), re::COUNT_COMMUNITIES),
    })
}

fn community_feed_post(article: ElementRef) -> Option<CommunityPost> {
    let url = article.value().attr(markers::COMMUNITY_POST_URL_ATTR)?;
    let (community_slug, slug) = community_post_ref(url)?;
    let (author, author_url) = user_link(article, sel::COMMUNITY_POST_AUTHOR);
    Some(CommunityPost {
        slug,
        id: attr(article, sel::COMMUNITY_POST_VOTE, markers::COMMUNITY_POST_ID_ATTR).and_then(|v| v.parse().ok()),
        community_slug,
        url: url.to_string(),
        title: first(article, sel::COMMUNITY_POST_TITLE).map(text_of).and_then(none_if_empty),
        author,
        author_url,
        date: first(article, sel::COMMUNITY_POST_DATE).map(text_of).and_then(none_if_empty),
        date_iso: None,
        text: first(article, sel::COMMUNITY_POST_TEXT).map(|e| rich_text(e, None)).unwrap_or_default(),
        flair: first(article, sel::COMMUNITY_POST_FLAIR).map(text_of).and_then(none_if_empty),
        perfumes: community_bottles(article),
        votes: first(article, sel::COMMUNITY_POST_SCORE).and_then(|e| text_of(e).parse().ok()),
        comments: None,
    })
}

/// A community and one page of its posts (`/communities/{slug}[?page=N]`, 20 posts).
pub fn parse_community(html: &str, requested_page: u32) -> PResult<CommunityPage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("community".into()));
    }
    let requested_page = requested_page.max(1);
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    require_footer(root, "community page")?;
    let Some(feed) = first(root, sel::COMMUNITY_FEED) else { return Err(outdated("community page: post feed not found")) };
    let name = first(root, sel::COMMUNITY_TITLE).map(text_of).and_then(none_if_empty);
    let canonical = attr(root, sel::CANONICAL, "href").unwrap_or_default();
    let (Some(name), Some(slug)) = (name, community_slug(canonical)) else {
        return Err(outdated("community page: name or canonical link not found"));
    };
    let mut items = Vec::new();
    for article in feed.select(&s(sel::COMMUNITY_POST)) {
        items.push(community_feed_post(article).ok_or_else(|| outdated("community page: post without post URL"))?);
    }
    let posts_total = root.select(&s(sel::COMMUNITY_STATS)).find_map(|e| count_in(&text_of(e), re::COUNT_POSTS));
    let community = Community {
        group_id: root.select(&s(sel::COMMUNITY_GROUP_ID)).find_map(|e| e.value().attr(markers::COMMUNITY_GROUP_ID_ATTR)?.parse().ok()),
        name,
        tagline: first(root, sel::COMMUNITY_TAGLINE).map(text_of).and_then(none_if_empty),
        description: first(root, sel::COMMUNITY_DESC).map(|e| rich_text(e, None)).and_then(none_if_empty),
        members: first(root, sel::COMMUNITY_MEMBERS).and_then(|e| count_in(&text_of(e), re::COUNT_MEMBERS)),
        posts: posts_total,
        access: first(root, sel::COMMUNITY_BADGE).map(text_of).and_then(none_if_empty),
        tags: root.select(&s(sel::COMMUNITY_TAGS)).map(|t| text_of(t).trim_start_matches('#').to_string()).filter(|t| !t.is_empty()).collect(),
        url: format!("{}{}", origin_of(canonical), routes::community(&slug, 1)),
        slug,
    };
    let posts = Listing {
        items,
        page: requested_page,
        total_pages: posts_total.map(|t| t.div_ceil(routes::COMMUNITY_POSTS_PER_PAGE).max(1)),
        total_items: posts_total,
    };
    Ok(CommunityPage { community, posts })
}

/// One community post (`/communities/{group}/post/{slug}`). The comments are not in the
/// anonymous page (the site loads them through a logged-in call): only their number is read.
pub fn parse_community_post(html: &str) -> PResult<CommunityPostPage> {
    if is_not_found_page(html) {
        return Err(ClientError::NotFound("community post".into()));
    }
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    require_footer(root, "community post page")?;
    let Some(detail) = first(root, sel::COMMUNITY_POST_DETAIL) else { return Err(outdated("community post page: post not found")) };
    let canonical = attr(root, sel::CANONICAL, "href").unwrap_or_default();
    let body = first(detail, sel::COMMUNITY_POST_DETAIL_BODY);
    // A post may be a title and a perfume only: it then has no text block (seen live at jalon 4).
    let Some((community_slug, slug)) = community_post_ref(canonical) else {
        return Err(outdated("community post page: canonical link not found"));
    };
    let origin = origin_of(canonical);
    let (author, author_url) = user_link(detail, sel::COMMUNITY_POST_DETAIL_AUTHOR);
    let metas: Vec<String> = detail.select(&s(sel::COMMUNITY_POST_DETAIL_META)).map(text_of).collect();
    let votes_re = rx(re::COUNT_VOTES);
    let community_link = first(root, sel::COMMUNITY_MINI_LINK);
    let community = CommunityRef {
        name: community_link.and_then(|a| first(a, sel::COMMUNITY_TITLE)).map(text_of).unwrap_or_default(),
        url: format!("{origin}{}", routes::community(&community_slug, 1)),
        slug: community_slug.clone(),
    };
    let post = CommunityPost {
        id: rx(re::COMMUNITY_POST_ID).captures(html).and_then(|c| c[1].parse().ok()),
        url: format!("{origin}{}", routes::community_post(&community_slug, &slug)),
        slug,
        community_slug,
        title: first(detail, sel::COMMUNITY_POST_DETAIL_TITLE).map(text_of).and_then(none_if_empty),
        author,
        author_url,
        date: metas.iter().find(|m| !m.is_empty() && !votes_re.is_match(m)).cloned(),
        date_iso: rx(re::JSONLD_DATE_PUBLISHED).captures(html).map(|c| c[1].to_string()),
        text: body.map(|b| rich_text(b, None)).unwrap_or_default(),
        flair: first(detail, sel::COMMUNITY_POST_FLAIR).map(text_of).and_then(none_if_empty),
        perfumes: community_bottles(detail),
        votes: metas.iter().find_map(|m| signed_count(m, re::COUNT_VOTES)),
        comments: count_in(html, re::COMMUNITY_COMMENT_COUNT),
    };
    Ok(CommunityPostPage { community, post, warnings: vec![markers::COMMUNITY_COMMENTS_ABSENT.to_string()] })
}
