//! Read-only access to the public forum and communities (jalon 4). Anonymous `GET`s
//! only, cached like the other listings.

use crate::{
    client::{LISTING_TTL, ParfumoClient},
    error::ClientError,
    models::*,
    parse,
    site::routes,
};

/// What a thread reference points at.
enum ThreadRef {
    /// Topic slug and the page number carried by the reference, if any.
    Topic(String, Option<u32>),
    /// Post id: the site shows the page of the thread that holds the post.
    Post(u64),
}

fn invalid(msg: &str) -> ClientError {
    ClientError::InvalidInput(msg.to_string())
}

/// Slugs of the forum and the communities: letters, digits and a few separators.
fn check_slug(slug: &str, what: &str) -> Result<String, ClientError> {
    let ok = !slug.is_empty() && slug.len() <= 200 && slug.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '%'));
    if ok { Ok(slug.to_string()) } else { Err(ClientError::InvalidInput(format!("not a valid {what} slug"))) }
}

impl ParfumoClient {
    /// Path segments of a reference and whether it was a URL or an absolute path.
    /// URLs must be on the Parfumo host (checked by [`Self::fetch_target`]).
    fn ref_segments(&self, r: &str) -> Result<(bool, Vec<String>), ClientError> {
        let r = r.trim();
        if r.is_empty() {
            return Err(invalid("empty reference"));
        }
        if r.chars().any(|c| c.is_control()) || r.contains("..") || r.contains('\\') {
            return Err(invalid("reference contains forbidden characters"));
        }
        let absolute = r.contains("://") || r.starts_with('/');
        let path = if absolute { self.fetch_target(r)? } else { r.to_string() };
        let segments = parse::url_path(&path).split('/').filter(|p| !p.is_empty()).map(str::to_string).collect();
        Ok((absolute, segments))
    }

    /// Categories of the forum, in site order, with their section, topic count and `forum_id`.
    pub async fn forum_categories(&self) -> Result<Vec<ForumCategory>, ClientError> {
        let (_, html) = self.t.get(routes::FORUMS, Some(LISTING_TTL), false, false).await?;
        parse::parse_forum_home(&html)
    }

    /// One page (50 topics) of a forum category. `category`: slug (`perfumes-brands`), path
    /// or URL (`/forums/perfumes-brands`). Sticky topics come first on page 1.
    pub async fn forum_topics(&self, category: &str, page: u32) -> Result<ForumTopicsPage, ClientError> {
        let hint = "expected a forum category slug or URL such as /forums/perfumes-brands";
        let (absolute, segments) = self.ref_segments(category)?;
        let mut parts = segments.iter().map(String::as_str).peekable();
        if absolute && parts.peek() != Some(&"forums") {
            return Err(invalid(hint));
        }
        if parts.peek() == Some(&"forums") {
            parts.next();
        }
        let slug = parts.next().ok_or_else(|| invalid(hint))?;
        if routes::FORUM_RESERVED.contains(&slug) {
            return Err(invalid(hint));
        }
        let slug = check_slug(slug, "forum category")?;
        let url_page = parts.next().and_then(|p| p.parse::<u32>().ok());
        let page = if page <= 1 { url_page.unwrap_or(1) } else { page };
        let (_, html) = self.t.get(&routes::forum_category(&slug, page), Some(LISTING_TTL), false, false).await?;
        parse::parse_forum_category(&html, page)
    }

    fn thread_ref(&self, topic: &str) -> Result<ThreadRef, ClientError> {
        let hint = "expected a topic slug, a topic URL (/forums/topic/{slug}) or a post URL (/forums/post/{id})";
        let (absolute, segments) = self.ref_segments(topic)?;
        let mut parts = segments.iter().map(String::as_str).peekable();
        if absolute && parts.peek() != Some(&"forums") {
            return Err(invalid(hint));
        }
        if parts.peek() == Some(&"forums") {
            parts.next();
        }
        match (parts.next(), parts.next(), parts.next()) {
            (Some("post"), Some(id), None) => id.parse().map(ThreadRef::Post).map_err(|_| invalid(hint)),
            (Some("topic"), Some(slug), page) => Ok(ThreadRef::Topic(check_slug(slug, "forum topic")?, page.and_then(|p| p.parse().ok()))),
            // A bare slug, possibly followed by a page number.
            (Some(slug), page, None) if !absolute => Ok(ThreadRef::Topic(check_slug(slug, "forum topic")?, page.and_then(|p| p.parse().ok()))),
            _ => Err(invalid(hint)),
        }
    }

    /// One page (25 posts) of a forum topic, with the topic head (`topic_id`, `forum_id`).
    /// `topic`: slug, topic URL or post URL (`/forums/post/{id}`). With a post URL and
    /// `page` ≤ 1 the page returned is the one holding that post (see `posts.page`).
    /// A page past the end is `NotFound` (the site answers 404).
    pub async fn forum_thread(&self, topic: &str, page: u32) -> Result<ForumThread, ClientError> {
        match self.thread_ref(topic)? {
            ThreadRef::Topic(slug, url_page) => {
                let page = if page <= 1 { url_page.unwrap_or(1) } else { page };
                let (_, html) = self.t.get(&routes::forum_topic(&slug, page), Some(LISTING_TTL), false, false).await?;
                parse::parse_forum_thread(&html, page)
            }
            ThreadRef::Post(id) => {
                let (_, html) = self.t.get(&routes::forum_post(id), Some(LISTING_TTL), false, false).await?;
                let thread = parse::parse_forum_thread(&html, 1)?;
                if page <= 1 || thread.posts.page == page {
                    return Ok(thread);
                }
                let path = routes::forum_topic(&thread.topic.slug, page);
                let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
                parse::parse_forum_thread(&html, page)
            }
        }
    }

    /// Forum search (`/s_forum_x.php?filter=…`): posts matching the text, newest first,
    /// 20 per page. The site shows at most 1000 results (50 pages).
    pub async fn forum_search(&self, query: &str, page: u32) -> Result<Listing<ForumSearchHit>, ClientError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(invalid("the search text must not be empty"));
        }
        if query.chars().count() > routes::FORUM_SEARCH_MAX_QUERY || query.chars().any(|c| c.is_control()) {
            return Err(ClientError::InvalidInput(format!(
                "the search text must be at most {} characters, without control characters",
                routes::FORUM_SEARCH_MAX_QUERY
            )));
        }
        let page = page.max(1);
        // The serializer is not `Send`: it must not live across the request.
        let path = {
            let mut params = url::form_urlencoded::Serializer::new(String::new());
            params.append_pair(routes::FORUM_SEARCH_TEXT_PARAM, query);
            if page > 1 {
                params.append_pair(routes::FORUM_SEARCH_PAGE_PARAM, &page.to_string());
            }
            format!("{}?{}", routes::FORUM_SEARCH, params.finish())
        };
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_forum_search(&html, page)
    }

    /// One page (12 communities) of the directory of communities (`/communities/discover`).
    pub async fn communities_page(&self, page: u32) -> Result<Listing<Community>, ClientError> {
        let page = page.max(1);
        let (_, html) = self.t.get(&routes::communities_index(page), Some(LISTING_TTL), false, false).await?;
        parse::parse_communities(&html, page)
    }

    /// Every community of the directory. The site lists 12 per page: this reads all the
    /// pages (one throttled request each on a cold cache, 9 pages when measured).
    pub async fn communities(&self) -> Result<Vec<Community>, ClientError> {
        let first = self.communities_page(1).await?;
        let pages = first.total_pages.unwrap_or(1).min(routes::COMMUNITIES_MAX_PAGES);
        let mut all = first.items;
        for page in 2..=pages {
            let next = self.communities_page(page).await?;
            // The site may answer a page past the end with an empty or repeated page.
            let before = all.len();
            for community in next.items {
                if !all.iter().any(|c| c.slug == community.slug) {
                    all.push(community);
                }
            }
            if all.len() == before {
                break;
            }
        }
        Ok(all)
    }

    fn community_slug(&self, group: &str) -> Result<String, ClientError> {
        let hint = "expected a community slug or URL such as /communities/scent-of-the-day";
        let (absolute, segments) = self.ref_segments(group)?;
        let mut parts = segments.iter().map(String::as_str).peekable();
        if absolute && parts.peek() != Some(&"communities") {
            return Err(invalid(hint));
        }
        if parts.peek() == Some(&"communities") {
            parts.next();
        }
        let slug = parts.next().ok_or_else(|| invalid(hint))?;
        if routes::COMMUNITY_RESERVED.contains(&slug) {
            return Err(invalid(hint));
        }
        check_slug(slug, "community")
    }

    /// A community (name, description, member count, `group_id`) and one page (20 posts,
    /// newest first) of its posts. `group`: slug, path or URL.
    pub async fn community(&self, group: &str, page: u32) -> Result<CommunityPage, ClientError> {
        let slug = self.community_slug(group)?;
        let page = page.max(1);
        let (_, html) = self.t.get(&routes::community(&slug, page), Some(LISTING_TTL), false, false).await?;
        parse::parse_community(&html, page)
    }

    /// One community post with its full text. `post`: the post slug (then `group` names
    /// the community), or the full URL / path of the post (then `group` is ignored and may
    /// be empty). The comments are not in the public page: only their number is returned.
    pub async fn community_post(&self, group: &str, post: &str) -> Result<CommunityPostPage, ClientError> {
        let hint = "expected a post slug, or a post URL such as /communities/{group}/post/{slug}";
        let (absolute, segments) = self.ref_segments(post)?;
        let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
        let (group_slug, post_slug) = match parts.as_slice() {
            ["communities", g, routes::COMMUNITY_POST_SEGMENT, p] => (check_slug(g, "community")?, *p),
            [g, routes::COMMUNITY_POST_SEGMENT, p] if !absolute => (check_slug(g, "community")?, *p),
            [routes::COMMUNITY_POST_SEGMENT, p] | [p] if !absolute => (self.community_slug(group)?, *p),
            _ => return Err(invalid(hint)),
        };
        if routes::COMMUNITY_RESERVED.contains(&group_slug.as_str()) {
            return Err(invalid(hint));
        }
        let post_slug = check_slug(post_slug, "community post")?;
        let path = routes::community_post(&group_slug, &post_slug);
        let (_, html) = self.t.get(&path, Some(LISTING_TTL), false, false).await?;
        parse::parse_community_post(&html)
    }
}
