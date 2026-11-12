//! Public community reads (jalon 4): blogs, the Souk (members' marketplace) and profile
//! walls. Read-only, anonymous, cached like the other listings.

use regex::Regex;
use reqwest::Url;

use crate::{
    client::{LISTING_TTL, ParfumoClient},
    error::ClientError,
    models::*,
    parse,
    site::{re, routes},
};

fn invalid(msg: &str) -> ClientError {
    ClientError::InvalidInput(msg.to_string())
}

impl ParfumoClient {
    /// Path of a caller reference (nickname, path or URL), with a leading `/` when the
    /// caller gave a path or a URL. A URL must point at the configured Parfumo host.
    fn community_ref(&self, r: &str) -> Result<String, ClientError> {
        let r = r.trim();
        if r.is_empty() {
            return Err(invalid("empty reference"));
        }
        if r.chars().any(|c| c.is_control() || c.is_whitespace()) || r.contains("..") || r.contains('\\') || r.contains('%') {
            return Err(invalid("reference contains forbidden characters"));
        }
        if r.contains("://") {
            let url = Url::parse(r).map_err(|_| invalid("not a valid URL"))?;
            let host = url.host_str().unwrap_or_default().trim_start_matches("www.");
            if !matches!(url.scheme(), "http" | "https") || host != self.t.cookie_domain() || !url.username().is_empty() || url.password().is_some() {
                return Err(ClientError::InvalidInput(format!("URL must be on {}", self.t.host())));
            }
            return Ok(url.path().trim_end_matches('/').to_string());
        }
        if r.starts_with("//") {
            return Err(invalid("protocol-relative URLs are not accepted"));
        }
        let cut = r.find(['?', '#']).unwrap_or(r.len());
        Ok(r[..cut].trim_end_matches('/').to_string())
    }

    /// Same, always as a `/Users/…` path: `nick/Blog/Article/slug` and
    /// `Users/nick/…` are completed.
    fn member_path(&self, r: &str, hint: &str) -> Result<String, ClientError> {
        let path = self.community_ref(r)?;
        let path = match path.strip_prefix('/') {
            Some(_) => path,
            None if path.starts_with(routes::USERS_PREFIX.trim_start_matches('/')) => format!("/{path}"),
            None => format!("{}{path}", routes::USERS_PREFIX),
        };
        if path.starts_with(routes::USERS_PREFIX) { Ok(path) } else { Err(invalid(hint)) }
    }

    /// Nickname of a member reference: a nickname, a `/Users/{nick}/…` path or a profile URL.
    fn market_member_nick(&self, r: &str) -> Result<String, ClientError> {
        let hint = "expected a Parfumo nickname or a member URL (/Users/{nick})";
        let path = self.member_path(r, hint)?;
        path[routes::USERS_PREFIX.len()..].split('/').next().filter(|nick| !nick.is_empty()).map(str::to_string).ok_or_else(|| invalid(hint))
    }

    /// One page of a site-wide blog feed. `Latest` and `News` have 8 articles per page;
    /// `Top` and `Hot` are the week's articles on a single page.
    pub async fn blog_feed(&self, feed: BlogFeed, page: u32) -> Result<Listing<BlogEntry>, ClientError> {
        let page = page.max(1);
        if page > 1 && matches!(feed, BlogFeed::Top | BlogFeed::Hot) {
            return Ok(Listing { items: Vec::new(), page, total_pages: Some(1), total_items: None });
        }
        let path = routes::with_page(routes::blog_feed(feed), page);
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_blog_feed(&html, page)
    }

    /// One page (6 articles) of the blog of a member, newest first. `nick`: nickname,
    /// `/Users/{nick}` path or profile URL. A member without article gives an empty list.
    pub async fn user_blog(&self, nick: &str, page: u32) -> Result<Listing<BlogEntry>, ClientError> {
        let page = page.max(1);
        let path = routes::with_page(&routes::user_blog(&self.market_member_nick(nick)?), page);
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_user_blog(&html, page)
    }

    /// One blog article with its full text and the comments shown in its page.
    /// `article`: article URL or path (`/Users/{nick}/Blog/Article/{slug}`).
    pub async fn blog_article(&self, article: &str) -> Result<BlogArticle, ClientError> {
        let hint = "expected a Parfumo blog article URL or path (/Users/{nick}/Blog/Article/{slug})";
        let path = self.member_path(article, hint)?;
        let url_re = Regex::new(re::BLOG_ARTICLE_URL).map_err(|_| invalid(hint))?;
        let ids = url_re.captures(&path).ok_or_else(|| invalid(hint))?;
        let path = routes::blog_article(&ids[1], &ids[2]);
        let (final_path, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_blog_article(&html, &self.t.absolute(parse::url_path(&final_path)))
    }

    /// Entries of the Souk, newest first. With a `query` (perfume or brand text) the site
    /// search is used: bottles and samples, 20 per page. Without one, the latest entries
    /// of the section are listed: bottles only for offers and requests, 16 per page.
    /// Lists carry no condition, size or price: read one entry with [`Self::souk_item`].
    pub async fn souk_search(&self, kind: SoukKind, query: Option<&str>, page: u32) -> Result<Listing<SoukItem>, ClientError> {
        let page = page.max(1);
        let path = match query.map(str::trim).filter(|q| !q.is_empty()) {
            None => routes::with_page(routes::souk_latest(kind), page),
            Some(q) => {
                if q.chars().count() > routes::SOUK_QUERY_MAX || q.chars().any(|c| c.is_control()) {
                    return Err(ClientError::InvalidInput(format!("query must be plain text of at most {} characters", routes::SOUK_QUERY_MAX)));
                }
                let mut params = url::form_urlencoded::Serializer::new(String::new());
                params
                    .append_pair("s_type", routes::souk_search_type(kind))
                    .append_pair("filter", q)
                    .append_pair("as", routes::SOUK_SEARCH_AS)
                    .append_pair("cs", routes::SOUK_SEARCH_CS)
                    .append_pair("img", routes::SOUK_SEARCH_IMG);
                if page > 1 {
                    params.append_pair("current_page", &page.to_string());
                }
                format!("{}?{}", routes::SOUK_SEARCH, params.finish())
            }
        };
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_souk_listing(&html, page)
    }

    /// Everything a member currently has in the Souk: offers and requests (bottles and
    /// samples) and running splits. One request per non-empty tab, at most
    /// `USER_SOUK_MAX_PAGES` requests. A member without entry gives an empty list.
    pub async fn user_souk(&self, nick: &str) -> Result<Vec<SoukItem>, ClientError> {
        let nick = self.market_member_nick(nick)?;
        let home = routes::user_souk(&nick);
        let mut queue = vec![home.clone()];
        let mut items: Vec<SoukItem> = Vec::new();
        let mut next = 0;
        while next < queue.len() && next < routes::USER_SOUK_MAX_PAGES {
            let path = queue[next].clone();
            next += 1;
            let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
            let (found, tabs, pages) = parse::parse_user_souk(&html)?;
            for item in found {
                if !items.iter().any(|i| i.id == item.id && i.kind == item.kind) {
                    items.push(item);
                }
            }
            // The site may spell the nickname with another case than the caller did.
            let own = home.to_lowercase();
            let mut more: Vec<String> = tabs.into_iter().filter(|tab| tab.to_lowercase().starts_with(&own)).collect();
            if !path.contains('?') {
                more.extend((2..=pages).map(|p| routes::with_page(&path, p)));
            }
            for path in more {
                if !queue.iter().any(|seen| seen.eq_ignore_ascii_case(&path)) {
                    queue.push(path);
                }
            }
        }
        Ok(items)
    }

    /// One Souk entry with its details (condition, size, price, shipping, note, photos).
    /// `item`: entry URL or path (`/Users/{nick}/Souk/Item/{id}`, `/Users/{nick}/Souk/Sharing/{id}`).
    pub async fn souk_item(&self, item: &str) -> Result<SoukItem, ClientError> {
        let hint = "expected a Parfumo Souk entry URL or path (/Users/{nick}/Souk/Item/{id} or /Users/{nick}/Souk/Sharing/{id})";
        let path = self.member_path(item, hint)?;
        let url_re = Regex::new(re::SOUK_ITEM_URL).map_err(|_| invalid(hint))?;
        let ids = url_re.captures(&path).ok_or_else(|| invalid(hint))?;
        let id = ids[3].parse::<u64>().map_err(|_| invalid(hint))?;
        let path = routes::souk_item(&ids[1], &ids[2], id);
        let (final_path, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_souk_item(&html, &self.t.absolute(parse::url_path(&final_path)))
    }

    /// One page (20 entries, newest first) of the wall of a member, with the owner's
    /// numeric id. `nick`: nickname, `/Users/{nick}` path or profile URL.
    pub async fn user_wall(&self, nick: &str, page: u32) -> Result<WallPage, ClientError> {
        let page = page.max(1);
        let path = routes::with_page(&routes::user_wall(&self.market_member_nick(nick)?), page);
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_wall(&html, page)
    }
}
