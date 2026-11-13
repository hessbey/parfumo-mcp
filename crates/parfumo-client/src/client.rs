//! High-level client: one method per database tool, plus session handling.

use std::{sync::Arc, time::Duration};

use reqwest::{Method, Url};

use crate::{
    error::ClientError,
    models::*,
    parse::{self, PerfumeContext},
    site::{markers, routes},
    transport::{ClientConfig, GlobalLimiter, PageCache, StoredCookie, Transport, UserLimiter, empty_body},
};

/// Cache lifetime of records (perfume, brand, perfumer, note, reviews, statements, similar).
pub const RECORD_TTL: Duration = Duration::from_secs(24 * 3600);
/// Cache lifetime of searches, rankings and arbitrary pages.
pub const LISTING_TTL: Duration = Duration::from_secs(3600);
const MAX_LINKS: usize = 100;

/// Client for one user: own cookie jar and own 1 request/s limiter, shared global limiter.
/// Cloning is cheap and shares the session.
#[derive(Clone)]
pub struct ParfumoClient {
    pub(crate) t: Arc<Transport>,
}

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

fn encode_query(params: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish()
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim()) || a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl ParfumoClient {
    pub fn new(cfg: ClientConfig, global: Arc<GlobalLimiter>, cache: Option<Arc<dyn PageCache>>) -> Result<Self, ClientError> {
        Self::with_limiter(cfg, global, cache, UserLimiter::new())
    }

    /// Like [`Self::new`], with a per-user limiter shared with other clients of that user.
    pub fn with_limiter(
        cfg: ClientConfig,
        global: Arc<GlobalLimiter>,
        cache: Option<Arc<dyn PageCache>>,
        limiter: Arc<UserLimiter>,
    ) -> Result<Self, ClientError> {
        Ok(Self { t: Arc::new(Transport::new(cfg, global, cache, limiter)?) })
    }

    /// Base URL in use, without trailing slash.
    pub fn base_url(&self) -> String {
        self.t.absolute("")
    }

    // -----------------------------------------------------------------------
    // Session
    // -----------------------------------------------------------------------

    /// Cookies of the current Parfumo session (to be stored encrypted by the caller).
    pub fn export_cookies(&self) -> Vec<StoredCookie> {
        self.t.jar.export()
    }

    /// Restores cookies previously exported. The memoised `csrf_key` is dropped.
    pub fn import_cookies(&self, c: &[StoredCookie]) {
        for cookie in c {
            self.t.jar.upsert(cookie.clone());
        }
        self.t.forget_csrf();
    }

    pub fn clear_cookies(&self) {
        self.t.jar.clear();
        self.t.forget_csrf();
    }

    /// Imports a `Cookie:` header value pasted from a browser
    /// (`pf_auth=…; PHPSESSID=…`). Returns the number of cookies imported.
    /// Measured (DISCOVERY §4): `pf_auth` alone is enough, the site then issues a new `PHPSESSID`.
    pub fn import_cookie_header(&self, header: &str) -> usize {
        let header = header.trim();
        let header = header.strip_prefix("Cookie:").or_else(|| header.strip_prefix("cookie:")).unwrap_or(header);
        let domain = self.t.cookie_domain();
        let mut count = 0;
        for part in header.split(';') {
            let Some((name, value)) = part.split_once('=') else { continue };
            let (name, value) = (name.trim(), value.trim());
            let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic() && c != ';' && c != ',');
            if !valid(name) || !valid(value) || name.contains('=') {
                continue;
            }
            self.t.jar.upsert(StoredCookie { name: name.to_string(), value: value.to_string(), domain: domain.clone(), path: "/".into() });
            count += 1;
        }
        if count > 0 {
            self.t.forget_csrf();
        }
        count
    }

    /// Logs in with `POST /api/account/login` (DISCOVERY §4). Never retried here.
    ///
    /// Measured: a wrong password answers `302` to `/account/login_error` → `InvalidCredentials`.
    ///
    /// Measured at jalon 0 bis: a success answers `302` to the home page and sets `pf_auth`
    /// plus a new `PHPSESSID`. When the redirect is not the error page but `pf_auth` is
    /// absent, a protected route is probed (`is_logged_in`) before answering.
    pub async fn login(&self, username: &str, password: &str) -> Result<(), ClientError> {
        if username.trim().is_empty() || password.is_empty() {
            return Err(ClientError::InvalidInput("username and password must not be empty".into()));
        }
        let form =
            vec![pair("username", username), pair("password", password), pair("autologin", "checked"), pair("login", "Login"), pair("redirect", "")];
        self.t.forget_csrf();
        let raw = self.t.send(Method::POST, routes::LOGIN, Some(&form), 0).await?;
        match raw.status {
            300..=399 => {
                let location = raw.location.unwrap_or_default();
                if location.contains(routes::LOGIN_ERROR_MARKER) || location.contains(routes::LOGIN_REDIRECT_MARKER) {
                    return Err(ClientError::InvalidCredentials);
                }
                if markers::LOGIN_COOKIES.iter().all(|c| self.t.jar.has(c)) {
                    return Ok(());
                }
                if self.is_logged_in().await? {
                    Ok(())
                } else {
                    Err(ClientError::Upstream("login was redirected but the session is not logged in".into()))
                }
            }
            200..=299 => {
                if parse::is_challenge(&raw.body) {
                    Err(ClientError::CaptchaRequired)
                } else if raw.body.contains(markers::INVALID_LOGIN) {
                    Err(ClientError::InvalidCredentials)
                } else if markers::LOGIN_COOKIES.iter().all(|c| self.t.jar.has(c)) {
                    Ok(())
                } else {
                    Err(ClientError::Upstream("login answered without redirect and without session cookies".into()))
                }
            }
            401 | 403 => Err(ClientError::InvalidCredentials),
            status => Err(ClientError::Upstream(format!("HTTP {status} on {}", routes::LOGIN))),
        }
    }

    /// Whether the session is logged in. One request to a protected route
    /// (`/account/notifications`): anonymous visitors get a `302` to the login page
    /// (measured); anything else than that redirect on a `2xx` is taken as "logged in"
    /// (NOT PROVEN without a test account).
    pub async fn is_logged_in(&self) -> Result<bool, ClientError> {
        let raw = self.t.send(Method::GET, routes::LOGGED_IN_PROBE, None, self.t.cfg.max_retries).await?;
        match raw.status {
            200..=299 => Ok(true),
            300..=399 => Ok(false),
            401 | 403 => Ok(false),
            status => Err(ClientError::Upstream(format!("HTTP {status} on {}", routes::LOGGED_IN_PROBE))),
        }
    }

    /// Session-bound `csrf_key` (`GET /api/alive`), memoised for the session.
    pub async fn csrf_key(&self) -> Result<String, ClientError> {
        self.t.csrf_key().await
    }

    // -----------------------------------------------------------------------
    // References
    // -----------------------------------------------------------------------

    /// Turns a user reference (absolute URL, absolute path or relative slug) into
    /// `(is_absolute_path, path)`. Absolute URLs must point at the configured host.
    pub(crate) fn ref_path(&self, r: &str) -> Result<(bool, String), ClientError> {
        let r = r.trim();
        if r.is_empty() {
            return Err(ClientError::InvalidInput("empty reference".into()));
        }
        if r.chars().any(|c| c.is_control()) || r.contains("..") || r.contains('\\') {
            return Err(ClientError::InvalidInput("reference contains forbidden characters".into()));
        }
        if r.contains("://") {
            let url = Url::parse(r).map_err(|_| ClientError::InvalidInput("not a valid URL".into()))?;
            let host = url.host_str().unwrap_or_default().trim_start_matches("www.");
            if !matches!(url.scheme(), "http" | "https") || host != self.t.cookie_domain() || !url.username().is_empty() || url.password().is_some() {
                return Err(ClientError::InvalidInput(format!("URL must be on {}", self.t.host())));
            }
            return Ok((true, url.path().to_string()));
        }
        let cut = r.find(['?', '#']).unwrap_or(r.len());
        let r = &r[..cut];
        if let Some(stripped) = r.strip_prefix('/') {
            Ok((true, format!("/{}", stripped.trim_end_matches('/'))))
        } else {
            Ok((false, r.trim_end_matches('/').to_string()))
        }
    }

    pub(crate) fn perfume_path(&self, r: &str) -> Result<String, ClientError> {
        let hint = "expected a Parfumo perfume URL or a 'Brand/Perfume' id as returned by parfumo_search_perfumes";
        let (absolute, path) = self.ref_path(r)?;
        let rel = if absolute {
            path.strip_prefix(routes::PERFUMES_PREFIX).ok_or_else(|| ClientError::InvalidInput(hint.into()))?.to_string()
        } else {
            path.strip_prefix("Perfumes/").unwrap_or(&path).to_string()
        };
        let mut parts = rel.split('/').filter(|p| !p.is_empty());
        match (parts.next(), parts.next()) {
            (Some(brand), Some(perfume)) => Ok(routes::perfume(&format!("{brand}/{perfume}"))),
            _ => Err(ClientError::InvalidInput(hint.into())),
        }
    }

    /// Record and page context (numeric id, tokens) of a perfume reference.
    pub(crate) async fn perfume_context(&self, r: &str) -> Result<(PerfumeDetails, PerfumeContext), ClientError> {
        self.perfume_ctx(r, false).await
    }

    async fn perfume_ctx(&self, r: &str, fresh: bool) -> Result<(PerfumeDetails, PerfumeContext), ClientError> {
        let path = self.perfume_path(r)?;
        let (_, html) = self.t.get(&path, Some(RECORD_TTL), fresh, false).await?;
        parse::parse_perfume_page(&html)
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    async fn resolve_filter_id(&self, route: &str, input: &str, id_attr: &str, what: &str, name: &str) -> Result<u64, ClientError> {
        let form = vec![pair("q", name.trim()), pair("input", input)];
        let body = self.t.post_fragment(route, &form, true, Some(RECORD_TTL)).await?;
        if body.trim().is_empty() {
            return Err(ClientError::NotFound(format!("{what} '{name}'")));
        }
        let found = parse::parse_label_filter(&body, id_attr);
        if let Some((id, _)) = found.iter().find(|(_, n)| same_name(n, name)) {
            return Ok(*id);
        }
        match found.as_slice() {
            [] => Err(ClientError::NotFound(format!("{what} '{name}'"))),
            [(id, _)] => Ok(*id),
            many => Err(ClientError::InvalidInput(format!(
                "{what} '{name}' is ambiguous; use one of: {}",
                many.iter().take(10).map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    /// Perfume search (`/s_perfumes_x.php`). 20 results per page; the site shows at most
    /// 1000 results. Brand, perfumer and note names are resolved to Parfumo ids first.
    pub async fn search_perfumes(&self, q: &SearchQuery, page: u32) -> Result<Listing<PerfumeSummary>, ClientError> {
        // Through the application interface when a session of it is held and the query can
        // be expressed there; the search of the web site otherwise.
        if self.has_app_session()
            && let Some(listing) = self.app_search_perfumes(q, page).await?
        {
            return Ok(listing);
        }
        let text = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let has_criteria = text.is_some()
            || q.brand.is_some()
            || q.perfumer.is_some()
            || !q.notes.is_empty()
            || q.year.is_some()
            || q.year_from.is_some()
            || q.year_to.is_some();
        if !has_criteria {
            return Err(ClientError::InvalidInput("give at least one of: text, brand, perfumer, notes, year, year_from, year_to".into()));
        }
        let mut params = vec![pair("v", "grid")];
        if let Some(text) = text {
            params.push(pair("filter", text));
        }
        if let Some(brand) = q.brand.as_deref().filter(|b| !b.trim().is_empty()) {
            let id = self.resolve_filter_id(routes::LABEL_FILTERS_BRANDS, routes::LABEL_INPUT_BRANDS, "data-b_id", "brand", brand).await?;
            params.push(pair("b_id", id));
        }
        if let Some(perfumer) = q.perfumer.as_deref().filter(|b| !b.trim().is_empty()) {
            let id =
                self.resolve_filter_id(routes::LABEL_FILTERS_PERFUMERS, routes::LABEL_INPUT_PERFUMERS, "data-pf_id", "perfumer", perfumer).await?;
            params.push(pair("pf_id", id));
        }
        let mut note_ids = Vec::new();
        for note in q.notes.iter().filter(|n| !n.trim().is_empty()) {
            let id = self.resolve_filter_id(routes::LABEL_FILTERS_NOTES, routes::LABEL_INPUT_NOTES, "data-n_inc_id", "note", note).await?;
            note_ids.push(id.to_string());
        }
        if !note_ids.is_empty() {
            params.push(pair("n_inc_id", note_ids.join(",")));
        }
        if let Some(year) = q.year {
            params.push(pair("y_t", 0));
            params.push(pair("y_0", year));
        } else if q.year_from.is_some() || q.year_to.is_some() {
            params.push(pair("y_t", 1));
            params.push(pair("y_1", q.year_from.unwrap_or(0)));
            params.push(pair("y_2", q.year_to.unwrap_or(0)));
        }
        for gender in &q.genders {
            let key = match gender {
                Gender::Men => "g_m",
                Gender::Women => "g_f",
                Gender::Unisex => "g_u",
            };
            if !params.iter().any(|(k, _)| k == key) {
                params.push(pair(key, 1));
            }
        }
        if let Some(sort) = routes::search_sort(q.sort) {
            params.push(pair("o", sort));
        }
        if page > 1 {
            params.push(pair("current_page", page));
        }
        let path = format!("{}?{}", routes::SEARCH, encode_query(&params));
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_listing(&html, page)
    }

    // -----------------------------------------------------------------------
    // Perfume record, reviews, statements, similar
    // -----------------------------------------------------------------------

    /// Full perfume record. `r` is an absolute URL, a path (`/Perfumes/Dior/Sauvage_Eau_de_Toilette`)
    /// or a `Brand/Perfume` id. The community classification needs a second request; if it
    /// fails the record is returned without it and a warning is added.
    pub async fn get_perfume(&self, r: &str) -> Result<PerfumeDetails, ClientError> {
        if self.has_app_session() {
            return self.app_get_perfume(r).await;
        }
        let (mut details, ctx) = self.perfume_ctx(r, false).await?;
        match &ctx.class_h {
            None => details.warnings.push("classification: token not found on the page".into()),
            Some(class_h) => {
                let form = vec![pair("type", "pie"), pair("p", ctx.p_id), pair("h", class_h)];
                let result = self.t.post_fragment(routes::GET_CLASSIFICATION_CHART, &form, true, Some(RECORD_TTL)).await.and_then(|body| {
                    if body.trim().is_empty() { Err(empty_body(routes::GET_CLASSIFICATION_CHART)) } else { parse::parse_classification(&body) }
                });
                match result {
                    Ok(groups) => details.classification = groups,
                    Err(e) => details.warnings.push(format!("classification unavailable: {e}")),
                }
            }
        }
        Ok(details)
    }

    /// Calls an `h`-bound endpoint of a perfume. On an empty body the perfume page is
    /// re-read without cache (the token may have changed) and the call replayed once.
    async fn perfume_fragment(
        &self,
        r: &str,
        ctx: PerfumeContext,
        route: &str,
        build: impl Fn(&PerfumeContext) -> Vec<(String, String)>,
    ) -> Result<(PerfumeContext, String), ClientError> {
        let body = self.t.post_fragment(route, &build(&ctx), false, Some(RECORD_TTL)).await?;
        if !body.trim().is_empty() {
            return Ok((ctx, body));
        }
        let (_, fresh) = self.perfume_ctx(r, true).await?;
        if fresh.h == ctx.h {
            return Ok((ctx, body));
        }
        let body = self.t.post_fragment(route, &build(&fresh), false, Some(RECORD_TTL)).await?;
        Ok((fresh, body))
    }

    /// Long reviews of a perfume, full text, `limit` ≤ 50 per call.
    pub async fn get_perfume_reviews(&self, r: &str, sort: ReviewSort, offset: u32, limit: u32) -> Result<Slice<Review>, ClientError> {
        if self.has_app_session()
            && let Some(slice) = self.app_perfume_reviews(r, sort, offset, limit).await?
        {
            return Ok(slice);
        }
        let n = limit.clamp(1, routes::MAX_SLICE);
        let (_, ctx) = self.perfume_ctx(r, false).await?;
        let total = ctx.reviews_total;
        if total.is_some_and(|t| offset >= t) {
            return Ok(Slice { items: Vec::new(), offset, total });
        }
        let (ctx, body) = self
            .perfume_fragment(r, ctx, routes::GET_REVIEWS, |c| {
                vec![
                    pair("p_id", c.p_id),
                    pair("f", offset),
                    pair("n", n),
                    pair("t", c.reviews_total.unwrap_or(0)),
                    pair("o", routes::review_sort(sort)),
                    pair("full", "Ja"),
                    pair("append", "false"),
                    pair("h", &c.h),
                ]
            })
            .await?;
        if body.trim().is_empty() {
            // With a known positive total an empty answer is a failure. Without any counter on
            // the page (perfume nobody wrote about yet) it is the site's way of saying "none".
            return match ctx.reviews_total {
                None => Ok(Slice { items: Vec::new(), offset, total: None }),
                Some(_) => Err(empty_body(routes::GET_REVIEWS)),
            };
        }
        let mut items = parse::parse_reviews(&body, &ctx.url)?;
        items.truncate(n as usize);
        Ok(Slice { items, offset, total: ctx.reviews_total })
    }

    /// Statements (short reviews) of a perfume, `limit` ≤ 50 per call.
    pub async fn get_perfume_statements(&self, r: &str, sort: ReviewSort, offset: u32, limit: u32) -> Result<Slice<Statement>, ClientError> {
        if self.has_app_session()
            && let Some(slice) = self.app_perfume_statements(r, sort, offset, limit).await?
        {
            return Ok(slice);
        }
        let n = limit.clamp(1, routes::MAX_SLICE);
        let (_, ctx) = self.perfume_ctx(r, false).await?;
        let total = ctx.statements_total;
        if total.is_some_and(|t| offset >= t) {
            return Ok(Slice { items: Vec::new(), offset, total });
        }
        let (ctx, body) = self
            .perfume_fragment(r, ctx, routes::GET_STATEMENTS, |c| {
                vec![
                    pair("p_id", c.p_id),
                    pair("f", offset),
                    pair("n", n),
                    pair("t", c.statements_total.unwrap_or(0)),
                    pair("o", routes::review_sort(sort)),
                    pair("append", "false"),
                    pair("h", &c.h),
                ]
            })
            .await?;
        if body.trim().is_empty() {
            // With a known positive total an empty answer is a failure. Without any counter on
            // the page (perfume nobody wrote about yet) it is the site's way of saying "none".
            return match ctx.statements_total {
                None => Ok(Slice { items: Vec::new(), offset, total: None }),
                Some(_) => Err(empty_body(routes::GET_STATEMENTS)),
            };
        }
        let mut items = parse::parse_statements(&body, &ctx.url)?;
        items.truncate(n as usize);
        Ok(Slice { items, offset, total: ctx.statements_total })
    }

    /// Similar perfumes, layering suggestions and dupes, with default options
    /// (URLs of the first 5 similar perfumes resolved).
    pub async fn get_similar_perfumes(&self, r: &str) -> Result<SimilarPerfumes, ClientError> {
        self.get_similar_perfumes_with(r, SimilarOptions::default()).await
    }

    async fn similar_list(&self, r: &str, kind: &str) -> Result<(PerfumeContext, Vec<SimilarPerfume>), ClientError> {
        let (_, ctx) = self.perfume_ctx(r, false).await?;
        let (ctx, body) = self
            .perfume_fragment(r, ctx, routes::GET_SIMILAR, |c| {
                vec![pair("p_id", c.p_id), pair("similar_type", kind), pair("type", "all"), pair("h", &c.h)]
            })
            .await?;
        if !body.trim().is_empty() {
            return Ok((ctx, parse::parse_similar(&body)?));
        }
        // An empty body is either "no entry" or a rejected call: the inspiration block
        // tells them apart (it is rendered even when the list is empty).
        let form = vec![pair("p_id", ctx.p_id), pair("type", kind), pair("h", &ctx.h)];
        let block = self.t.post_fragment(routes::GET_INSPIRATION, &form, false, Some(RECORD_TTL)).await?;
        if block.trim().is_empty() {
            return Err(empty_body(routes::GET_SIMILAR));
        }
        Ok((ctx, parse::parse_similar(&block)?))
    }

    /// Similar perfumes with explicit options. The site lists similar perfumes without
    /// links; `resolve_urls` entries are resolved through `similar_sneak_peek.php`
    /// (one request each, cached 24 h).
    pub async fn get_similar_perfumes_with(&self, r: &str, opts: SimilarOptions) -> Result<SimilarPerfumes, ClientError> {
        // The two lists through the application interface when a session of it is held and
        // it finds the perfume; all of it from the web site otherwise.
        if self.has_app_session()
            && let Some(found) = self.app_similar_perfumes(r, opts).await?
        {
            return Ok(found);
        }
        let (ctx, mut similar) = self.similar_list(r, "similar").await?;
        let mut warnings = Vec::new();

        for item in similar.iter_mut().take(opts.resolve_urls) {
            let form = vec![pair("type", "similar"), pair("p_id", ctx.p_id), pair("s_id", item.p_id)];
            let resolved =
                self.t.post_fragment(routes::SIMILAR_SNEAK_PEEK, &form, true, Some(RECORD_TTL)).await.and_then(|body| {
                    if body.trim().is_empty() { Err(empty_body(routes::SIMILAR_SNEAK_PEEK)) } else { parse::parse_sneak_peek(&body) }
                });
            match resolved {
                Ok((url, votes)) => {
                    item.id = parse::perfume_id_from_url(&url);
                    item.url = Some(url);
                    item.votes = votes;
                }
                Err(e) => {
                    warnings.push(format!("links of similar perfumes not resolved: {e}"));
                    break;
                }
            }
        }

        let mut layering = Vec::new();
        if opts.include_layering {
            match self.similar_list(r, "layer").await {
                Ok((_, list)) => layering = list,
                Err(e) => warnings.push(format!("layering suggestions unavailable: {e}")),
            }
        }

        let mut dupes = Vec::new();
        if opts.include_dupes {
            let path = format!("{}?{}", routes::GET_DUPES, encode_query(&[pair("q", &ctx.name)]));
            let found = self.t.get(&path, Some(RECORD_TTL), false, true).await.and_then(|(_, body)| parse::parse_dupes(&body));
            match found {
                Ok(groups) => {
                    dupes = groups.into_iter().filter(|g| g.original.id == ctx.id || g.dupes.iter().any(|d| d.perfume.id == ctx.id)).collect();
                }
                Err(e) => warnings.push(format!("dupes directory unavailable: {e}")),
            }
        }

        Ok(SimilarPerfumes { perfume_id: ctx.id, similar, layering, dupes, warnings })
    }

    // -----------------------------------------------------------------------
    // Brand, perfumer, note
    // -----------------------------------------------------------------------

    async fn livesearch(&self, route: &str, name: &str) -> Result<String, ClientError> {
        let form = vec![pair("q", name.trim())];
        self.t.post_fragment(route, &form, true, Some(RECORD_TTL)).await
    }

    /// Resolves an entity reference to a page: explicit URL / path, else the slug guessed
    /// from the name, else the site's own autocompletion.
    async fn entity_page(&self, r: &str, prefix: &str, livesearch_route: &str, page: u32, what: &str) -> Result<String, ClientError> {
        let (absolute, path) = self.ref_path(r)?;
        if absolute {
            let slug = path
                .strip_prefix(prefix)
                .and_then(|rest| rest.split('/').find(|p| !p.is_empty()))
                .ok_or_else(|| ClientError::InvalidInput(format!("not a Parfumo {what} URL")))?;
            let (_, html) = self.t.get(&routes::with_page(&format!("{prefix}{slug}"), page), Some(RECORD_TTL), false, false).await?;
            return Ok(html);
        }
        let name = path.strip_prefix(prefix.trim_start_matches('/')).unwrap_or(&path);
        let name = name.split('/').next().unwrap_or(name);
        let guess = format!("{prefix}{}", name.trim().replace(' ', "_"));
        match self.t.get(&routes::with_page(&guess, page), Some(RECORD_TTL), false, false).await {
            Ok((_, html)) => return Ok(html),
            Err(ClientError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let query = name.replace('_', " ");
        let body = self.livesearch(livesearch_route, &query).await?;
        let found = parse::parse_livesearch_links(&body);
        let url = found
            .iter()
            .find(|(n, _)| same_name(n, &query))
            .or(found.first())
            .map(|(_, url)| url.clone())
            .ok_or_else(|| ClientError::NotFound(format!("{what} '{query}'")))?;
        let target = parse::url_path(&url).to_string();
        if !target.starts_with(prefix) {
            return Err(ClientError::NotFound(format!("{what} '{query}'")));
        }
        let (_, html) = self.t.get(&routes::with_page(&target, page), Some(RECORD_TTL), false, false).await?;
        Ok(html)
    }

    /// Brand record and one page (20 items) of its perfumes. `r` is a URL, a path
    /// (`/Perfumes/Dior`), a slug or a brand name.
    pub async fn get_brand(&self, r: &str, page: u32) -> Result<BrandPage, ClientError> {
        let html = self.entity_page(r, routes::PERFUMES_PREFIX, routes::LIVESEARCH_BRAND, page, "brand").await?;
        parse::parse_brand_page(&html, page)
    }

    /// Perfumer record and one page (20 items) of their creations. `r` is a URL, a path
    /// (`/Perfumers/…`), a slug or a perfumer name.
    pub async fn get_perfumer(&self, r: &str, page: u32) -> Result<PerfumerPage, ClientError> {
        let html = self.entity_page(r, routes::PERFUMERS_PREFIX, routes::LIVESEARCH_PERFUMER, page, "perfumer").await?;
        parse::parse_perfumer_page(&html, page)
    }

    /// Fragrance note (or note category) and one page (20 items) of perfumes containing it.
    /// `r` is a URL, a path (`/Fragrance_Note/Bergamot` for a note, `/Fragrance_Notes/Citric_notes`
    /// for a category), a slug or a note name.
    pub async fn get_note(&self, r: &str, page: u32) -> Result<NotePage, ClientError> {
        let (absolute, path) = self.ref_path(r)?;
        if absolute {
            let (prefix, category) = if path.starts_with(routes::NOTE_CATEGORY_PREFIX) {
                (routes::NOTE_CATEGORY_PREFIX, true)
            } else if path.starts_with(routes::NOTE_PREFIX) {
                (routes::NOTE_PREFIX, false)
            } else {
                return Err(ClientError::InvalidInput("not a Parfumo fragrance note URL".into()));
            };
            let slug = path[prefix.len()..]
                .split('/')
                .find(|p| !p.is_empty())
                .ok_or_else(|| ClientError::InvalidInput("fragrance note URL without a note".into()))?;
            let (_, html) = self.t.get(&routes::with_page(&routes::note(slug, category), page), Some(RECORD_TTL), false, false).await?;
            return parse::parse_note_page(&html, page);
        }
        let name = path.trim();
        let guess = routes::note(&name.replace(' ', "_"), false);
        match self.t.get(&routes::with_page(&guess, page), Some(RECORD_TTL), false, false).await {
            Ok((_, html)) => return parse::parse_note_page(&html, page),
            Err(ClientError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let query = name.replace('_', " ");
        let body = self.livesearch(routes::LIVESEARCH_NOTE, &query).await?;
        let found = parse::parse_livesearch_notes(&body);
        let slug = found
            .iter()
            .find(|(n, _)| same_name(n, &query))
            .or(found.first())
            .map(|(_, slug)| slug.clone())
            .ok_or_else(|| ClientError::NotFound(format!("fragrance note '{query}'")))?;
        let (_, html) = self.t.get(&routes::with_page(&routes::note(&slug, false), page), Some(RECORD_TTL), false, false).await?;
        parse::parse_note_page(&html, page)
    }

    // -----------------------------------------------------------------------
    // Rankings
    // -----------------------------------------------------------------------

    /// Rankings. Page sizes: top lists 100 (5 pages), recently added and release year 20
    /// (at most 1000 results), trends 20 on a single page.
    pub async fn get_rankings(&self, kind: &RankingKind, page: u32) -> Result<Listing<PerfumeSummary>, ClientError> {
        let page = page.max(1);
        if let RankingKind::Year(y) = kind
            && !(1700..=2100).contains(y)
        {
            return Err(ClientError::InvalidInput("year out of range".into()));
        }
        // Through the application interface when a session of it is held and it serves
        // this ranking; the web pages otherwise.
        if self.has_app_session()
            && let Some(listing) = self.app_rankings(kind, page).await?
        {
            return Ok(listing);
        }
        let path = routes::ranking(kind);
        if let Some(section) = routes::trends_section(kind) {
            let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
            let mut sections = parse::parse_trends(&html)?;
            if section >= sections.len() {
                return Err(ClientError::ParserOutdated("trends page: expected three ranked sections".into()));
            }
            let (_, items) = sections.swap_remove(section);
            let total = items.len() as u32;
            let items = if page > 1 { Vec::new() } else { items };
            return Ok(Listing { items, page, total_pages: Some(1), total_items: Some(total) });
        }
        let (_, html) = self.t.get(&routes::with_page(&path, page), Some(LISTING_TTL), false, false).await?;
        parse::parse_listing(&html, page)
    }

    // -----------------------------------------------------------------------
    // Fallback page reader
    // -----------------------------------------------------------------------

    /// Validates a `fetch_page` target and returns its path + query on the base host.
    pub fn fetch_target(&self, path_or_url: &str) -> Result<String, ClientError> {
        let raw = path_or_url.trim();
        if raw.is_empty() || raw.chars().any(|c| c.is_control() || c == '\\') {
            return Err(ClientError::InvalidInput("empty or malformed page reference".into()));
        }
        let url = if raw.contains("://") {
            Url::parse(raw).map_err(|_| ClientError::InvalidInput("not a valid URL".into()))?
        } else if raw.starts_with("//") {
            return Err(ClientError::InvalidInput("protocol-relative URLs are not accepted".into()));
        } else {
            let path = if raw.starts_with('/') { raw.to_string() } else { format!("/{raw}") };
            self.t.base.join(&path).map_err(|_| ClientError::InvalidInput("not a valid path".into()))?
        };
        let host_ok = url.host_str().is_some_and(|h| h == self.t.host() || h.trim_start_matches("www.") == self.t.cookie_domain());
        if !matches!(url.scheme(), "http" | "https")
            || !host_ok
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != self.t.base.port_or_known_default()
        {
            return Err(ClientError::InvalidInput(format!("only pages of {} can be fetched", self.t.host())));
        }
        check_fetch_path(url.path())?;
        Ok(match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        })
    }

    /// Fallback: reads any public page of the site (GET only) and returns its cleaned
    /// text. Internal interfaces (`/api/`, `/action/`) and account-critical routes are refused.
    pub async fn fetch_page(&self, path_or_url: &str) -> Result<FetchedPage, ClientError> {
        let target = self.fetch_target(path_or_url)?;
        // A redirect is checked before it is followed: a page that points to an internal or
        // account-critical route is refused without that route being requested.
        let (final_path, html) = self.t.get_checked(&target, Some(LISTING_TTL), false, false, &check_fetch_path).await?;
        check_fetch_path(parse::url_path(&final_path))?;
        parse::parse_page_text(&html, &self.t.absolute(&final_path), MAX_LINKS)
    }
}

/// Refuses internal and account-critical paths, including percent-encoded variants.
fn check_fetch_path(path: &str) -> Result<(), ClientError> {
    let decoded = percent_decode(path).to_ascii_lowercase();
    let mut normal = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        if c == '/' && normal.ends_with('/') {
            continue;
        }
        normal.push(c);
    }
    if normal.contains("..") || normal.contains('\\') || normal.contains('%') {
        return Err(ClientError::InvalidInput("path contains forbidden sequences".into()));
    }
    for prefix in routes::FETCH_FORBIDDEN_PREFIXES {
        let hit = if prefix.ends_with('/') || prefix.ends_with(".php") {
            normal.starts_with(prefix)
        } else {
            normal == *prefix
                || normal.starts_with(&format!("{prefix}/"))
                || normal.starts_with(&format!("{prefix}_"))
                || (prefix.starts_with("/account/") && normal.starts_with(prefix))
        };
        if hit {
            return Err(ClientError::InvalidInput("this path is not readable with fetch_page (internal interface or account-critical route)".into()));
        }
    }
    Ok(())
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&path[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Exploration hook (feature `explore`, jalon 0 bis). Not compiled in the server.
#[cfg(feature = "explore")]
impl ParfumoClient {
    /// Sends one throttled request and returns the raw answer. No retry, no interpretation
    /// besides challenge detection (`CaptchaRequired`).
    pub async fn explore_request(
        &self,
        method: &str,
        path_and_query: &str,
        form: Option<&[(String, String)]>,
        raw_body: Option<(String, Vec<u8>)>,
        extra_headers: &[(String, String)],
    ) -> Result<crate::transport::ExploreResponse, ClientError> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| ClientError::InvalidInput("invalid HTTP method".into()))?;
        self.t.explore(method, path_and_query, form, raw_body, extra_headers).await
    }
}
