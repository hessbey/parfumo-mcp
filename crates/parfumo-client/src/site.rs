//! Everything that depends on the current shape of parfumo.com lives here and
//! nowhere else: routes (URL builders), CSS selectors, regular expressions and
//! text markers. When the site changes, this is the only module to update; the
//! canary test and the fixtures exercise it through `parse.rs`.
//!
//! Source of truth: `docs/DISCOVERY.md` plus the pages captured in `tests/fixtures`.

/// URL builders. All paths are relative to the configured base URL.
pub mod routes {
    use crate::models::{RankingKind, ReviewSort, SearchSort};

    /// `GET` → `{"csrf_key":"…"}` (JSON served as `text/html`).
    pub const ALIVE: &str = "/api/alive";
    /// `POST` login form target.
    pub const LOGIN: &str = "/api/account/login";
    /// Where anonymous visitors are redirected by protected routes.
    pub const LOGIN_REDIRECT_MARKER: &str = "/action/dologin";
    /// Where a failed login redirects.
    pub const LOGIN_ERROR_MARKER: &str = "/account/login_error";
    /// A protected route used as a logged-in oracle (302 to the login page when anonymous).
    pub const LOGGED_IN_PROBE: &str = "/account/notifications";

    pub const PERFUMES_PREFIX: &str = "/Perfumes/";
    pub const PERFUMERS_PREFIX: &str = "/Perfumers/";
    pub const NOTE_PREFIX: &str = "/Fragrance_Note/";
    pub const NOTE_CATEGORY_PREFIX: &str = "/Fragrance_Notes/";
    pub const USERS_PREFIX: &str = "/Users/";

    pub const SEARCH: &str = "/s_perfumes_x.php";
    pub const GET_REVIEWS: &str = "/action/perfume/get_reviews.php";
    pub const GET_STATEMENTS: &str = "/action/perfume/get_statements.php";
    pub const GET_SIMILAR: &str = "/action/perfume/get_similar.php";
    pub const GET_INSPIRATION: &str = "/action/perfume/get_inspiration.php";
    pub const SIMILAR_SNEAK_PEEK: &str = "/action/perfume/similar_sneak_peek.php";
    pub const GET_CLASSIFICATION_CHART: &str = "/action/perfume/get_classification_chart.php";
    pub const GET_DUPES: &str = "/action/perfume/_get_dupes.php";
    pub const LABEL_FILTERS_BRANDS: &str = "/action/s_x/label_filters_brands.php";
    pub const LABEL_FILTERS_PERFUMERS: &str = "/action/s_x/label_filters_perfumers.php";
    pub const LABEL_FILTERS_NOTES: &str = "/action/s_x/label_filters_notes.php";
    pub const LIVESEARCH_BRAND: &str = "/action/livesearch/livesearch_brand.php";
    pub const LIVESEARCH_PERFUMER: &str = "/action/livesearch/livesearch_perfumer.php";
    pub const LIVESEARCH_NOTE: &str = "/action/livesearch/livesearch_note.php";

    /// `input` parameter expected by each `label_filters_*` endpoint.
    pub const LABEL_INPUT_BRANDS: &str = "b_id_input";
    pub const LABEL_INPUT_PERFUMERS: &str = "pf_id_input";
    pub const LABEL_INPUT_NOTES: &str = "n_inc_id_input";

    // --- account: collection (DISCOVERY §3.2, §5), confirmed logged in at jalon 0 bis ---
    /// `GET` ShelfControl page of the default list; `/{slug or id}` for another list.
    pub const SHELFCONTROL: &str = "/account/shelfcontrol";
    pub const WARDROBE_CSV: &str = "/action/wardrobe/generate_wardrobe_csv.php";
    pub const WARDROBE_ADD_REMOVE: &str = "/api/wardrobe/wardrobe_add_remove";
    pub const WARDROBE_COPY_MOVE: &str = "/api/wardrobe/wardrobe_copy_move";
    pub const WARDROBE_UPDATE_DETAILS: &str = "/api/wardrobe/update_details";
    pub const SHELF_COLUMNS: &str = "/action/shelfcontrol/columns.php";
    pub const MEMO_CREATE: &str = "/api/memo/create_memo";
    pub const MEMO_DELETE: &str = "/api/memo/delete_memo";
    pub const WARDROBE_ADD_LIST: &str = "/api/wardrobe/add_list";
    pub const WARDROBE_SET_NAME: &str = "/api/wardrobe/set_wardrobe_name_notes";
    pub const WARDROBE_LOCK_TOGGLE: &str = "/api/wardrobe/wardrobe_lock_toggle";
    pub const WARDROBE_DELETE_LIST: &str = "/api/wardrobe/del_cust_wr";
    /// Highest id of the fixed lists (1 have … 7 mini); custom lists have larger ids.
    pub const LAST_FIXED_LIST_ID: u64 = 7;
    pub const LIST_HAVE: u64 = 1;
    pub const LIST_HAD: u64 = 2;
    pub const LIST_NAME_MAX: usize = 40;
    pub const LIST_NOTES_MAX: usize = 120;

    // --- account: wears (DISCOVERY §3.3) ---
    pub const WEAR_LOG: &str = "/api/assistant/i_wear";
    pub const WEAR_CALENDAR_MONTH: &str = "/api/assistant/calendar_month";
    pub const WEAR_TRACKER: &str = "/assistant/tracker";
    pub const WEAR_SET_DATE: &str = "/api/account/update_iwear_date";
    pub const WEAR_SET_COMMENT: &str = "/api/account/update_iwear_comment";
    pub const WEAR_DELETE: &str = "/api/account/delete_iwear";
    pub const WEAR_COMMENT_MAX: usize = 255;

    // --- account: ratings, classification, reviews, statements (DISCOVERY §3.4, §3.5, §11) ---
    /// `POST type, p_id, v` (0-100, the site shows v/10) → `{"status":"ok"}`. Confirmed logged in.
    pub const VOTE_ADD: &str = "/api/perfume/add_vote";
    pub const VOTE_REMOVE: &str = "/api/perfume/remove_vote";
    /// `POST perfume_id` → fragment with the member's five ratings (stars out of 5).
    pub const RATE_PANEL: &str = "/action/perfume/perfume_rate_panel.php";
    /// `POST perfume_id` → fragment with every classification label and the member's votes.
    pub const CLASSIFY_PANEL: &str = "/action/perfume/perfume_classify_panel.php";
    /// `POST p, c, ct, action` (`add` / `remove`) → `1` / `0`. Confirmed logged in.
    pub const CLASSIFY_TOGGLE: &str = "/api/perfume/toggle_classification_vote";
    /// `POST perfume_id, review_id` → the review form (delete button when a review exists).
    pub const REVIEW_PANEL: &str = "/action/perfume/perfume_review_panel.php";
    pub const STATEMENT_PANEL: &str = "/action/perfume/perfume_statement_panel.php";
    /// `POST p, lang` → `{"review_title":"","review_text":""}`. Confirmed logged in (empty state).
    pub const REVIEW_GET_OWN: &str = "/api/review/get_user_review";
    /// `POST p, lang` → `{"statement_text":""}`. Confirmed logged in (empty state).
    pub const STATEMENT_GET_OWN: &str = "/api/statement/get_user_statement";
    /// Seen in the site script, never sent: `{"status":0,"msg":…}` on refusal, else `url`.
    pub const REVIEW_CREATE: &str = "/api/review/create_review";
    pub const REVIEW_DELETE: &str = "/api/review/delete_review";
    pub const STATEMENT_CREATE: &str = "/api/statement/create_statement";
    pub const STATEMENT_DELETE: &str = "/api/statement/delete_statement";
    pub const STATEMENT_MAX: usize = 140;
    pub const REVIEW_TITLE_MAX: usize = 200;
    pub const REVIEW_TEXT_MAX: usize = 7_500;
    /// Language sent with the `get_user_*` reads (the English site).
    pub const OWN_TEXT_LANG: &str = "en";

    // --- jalon 4: profile, mailbox, notifications, follows, settings (DISCOVERY §3.6, §3.7, §12) ---
    /// `POST` → fragment of a member's collection list (`div.war_it` per perfume). Public.
    pub const GET_WARDROBE: &str = "/action/wardrobe/get_wardrobe.php";
    /// `POST id` (entry id) → name, brand and link of one collection entry. Public.
    pub const WARDROBE_SNEAK_PEEK: &str = "/action/wardrobe/wardrobe_sneak_peek.php";
    /// `GET ?n=` → conversations of the mailbox (first 25).
    pub const POSTBOX_CONVERSATIONS: &str = "/action/postbox/_get_postbox_conv.php";
    /// `GET ?n=&f=25` → the following conversations, 25 at a time; empty at the end.
    pub const POSTBOX_CONVERSATIONS_MORE: &str = "/action/postbox/_get_postbox_conv_more.php";
    pub const POSTBOX_PAGE_SIZE: u32 = 25;
    /// `GET ?r={member id}&before=&before_date=&l=` → message rows of a conversation.
    pub const POSTBOX_MESSAGES: &str = "/action/postbox/_get_postbox.php";
    /// `POST user_id, msg, temp_images` → `1`; anything else is the refusal text.
    pub const POSTBOX_SEND: &str = "/action/postbox/postbox_send.php";
    pub const MESSAGE_MAX: usize = 10_000;
    /// `GET` → fragment with the pending notifications.
    pub const NOTIFICATIONS: &str = "/api/notification/get_history";
    /// `POST operation (add / remove), item_id, type=users, sub_type=follow` → empty body.
    pub const SUBSCRIPTION: &str = "/api/subscription";
    /// `POST type, item_id` → JSON array of the sub types subscribed (`follow`, `reviews`…).
    pub const SUBSCRIPTION_SUB_TYPES: &str = "/api/subscription_sub_types";
    /// `GET` → members the account follows.
    pub const FOLLOW_LIST: &str = "/account/follow/list";
    pub const SETTINGS_PERSONAL: &str = "/account/personal_details";
    pub const SETTINGS_NOTIFICATIONS: &str = "/account/notifications";
    pub const SETTINGS_WEBSITE: &str = "/account/website";
    /// `POST a (field), v (true / false / option value)` → empty body, whatever the field.
    pub const SETTING_SET: &str = "/action/_aa_account.php";
    /// `GET` → form with the two fragrance preference lines.
    pub const FRAGRANCE_PREFERENCES: &str = "/account/fragrance_preferences";
    /// `POST do_edit=1, i_like, i_dont_like` → 302 to the profile.
    pub const FRAGRANCE_PREFERENCES_SAVE: &str = "/fragrance_preferences.php";
    pub const PREFERENCE_MAX: usize = 250;
    /// `POST p` (`p_id` or `clear`) → fragment of the profile sidebar.
    pub const SIGNATURE_SCENT: &str = "/api/account/signature_scent";
    /// Settings of `/account/personal_details`: they show on the public profile.
    pub const PROFILE_FIELDS: &[&str] = &["gender", "country_id", "time_zone_id"];
    /// `POST a (type), id, comment_id, text, p1, p2, redirect, l, h` → JSON `{type, val, redirect}`;
    /// `val` is empty on success. Seen in the script, never sent.
    pub const COMMENT_POST: &str = "/action/comments/_post_comment.php";
    /// Comment type of a profile wall.
    pub const COMMENT_TYPE_WALL: &str = "id_cards";
    /// `POST forum_id, subject, message` → JSON `{success, error, redirect}`. Seen, never sent.
    pub const FORUM_CREATE_TOPIC: &str = "/api/forum/create_topic";
    /// `POST topic_id, message` → same JSON. Seen, never sent.
    pub const FORUM_REPLY: &str = "/api/forum/reply";
    pub const FORUM_SUBJECT_MAX: usize = 60;
    /// `POST forum_id` → the "new topic" form of a forum category.
    pub const FORUM_COMPOSE_PANEL: &str = "/action/forum/compose_panel.php";
    /// `POST blog_write_post=1, blog_nick, blog_title, blog_text` ("Save & Preview"). Seen, never sent.
    pub const BLOG_POST: &str = "/action/blogs/post_blog.php";
    pub const BLOG_TITLE_MAX: usize = 200;
    /// `POST perfume_id, t, souk_id` → form of a Souk entry (or "Not available").
    pub const SOUK_ITEM_PANEL: &str = "/action/souk/souk_item_panel.php";
    /// `POST` of the form → JSON `{result: "success", id, url}` or `{errors, error}`. Seen, never sent.
    pub const SOUK_ADD: &str = "/api/souk/souk_adder";
    pub const SOUK_UPDATE: &str = "/api/souk/souk_update";

    pub fn user(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}")
    }

    pub fn user_collection(nick: &str, list: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Collection/{list}")
    }

    pub fn postbox_thread(nick: &str) -> String {
        format!("/Postbox/R/{nick}")
    }
    /// The five rating scales, as the site names them.
    pub const RATING_SCALES: &[&str] = &["scent", "durability", "sillage", "bottle", "pricing"];

    /// Ratings the member gave on one scale. `order`: `d_desc`, `d_asc`, `v_desc`, `v_asc`.
    pub fn user_ratings(nick: &str, scale: &str, order: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Ratings/{scale}?order={order}")
    }

    pub fn user_reviews(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}/reviews")
    }

    pub fn user_statements(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}/statements")
    }

    pub fn shelfcontrol(list: &str) -> String {
        format!("{SHELFCONTROL}/{list}")
    }

    pub fn wardrobe_csv(list_id: u64) -> String {
        format!("{WARDROBE_CSV}?list_id={list_id}")
    }

    // --- community reads: blogs, Souk, wall (jalon 4), measured anonymous on 2026-11-12 ---
    /// Latest blog articles of the whole site, 8 per page (`?current_page=N`).
    pub const BLOGS: &str = "/blogs";
    /// Latest Souk entries: bottles only, 16 per page (`?current_page=N`). Samples live
    /// under `{…}/Samples` and are reached through the search instead.
    pub const SOUK_OFFERS: &str = "/Souks/Offers";
    pub const SOUK_REQUESTS: &str = "/Souks/Requests";
    pub const SOUK_SHARINGS: &str = "/Souks/Sharings";
    /// `GET filter, s_type, as, cs, img` → 20 entries per page (`current_page`). An empty
    /// `filter` finds nothing. `as` / `cs` are hidden fields of the form, always `1`; sending
    /// `0` changes nothing. `img=0` keeps only entries with a photo, `img=1` lists them all.
    /// `fl` (bottles) and `ab` (samples) are both on when absent.
    pub const SOUK_SEARCH: &str = "/s_souk.php";
    pub const SOUK_SEARCH_AS: &str = "1";
    pub const SOUK_SEARCH_CS: &str = "1";
    pub const SOUK_SEARCH_IMG: &str = "1";
    pub const SOUK_QUERY_MAX: usize = 160;
    /// Pages read at most for one member's Souk (one per non-empty tab, plus further pages).
    pub const USER_SOUK_MAX_PAGES: usize = 8;
    /// Tab of a member's Souk that is not read: splits that are over.
    pub const USER_SOUK_CLOSED: &str = "SharingsClosed";

    pub fn blog_feed(feed: crate::models::BlogFeed) -> &'static str {
        use crate::models::BlogFeed;
        match feed {
            BlogFeed::Latest => BLOGS,
            BlogFeed::News => "/blogs/news",
            BlogFeed::Top => "/blogs/top",
            BlogFeed::Hot => "/blogs/hot",
        }
    }

    pub fn user_blog(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Blog")
    }

    pub fn blog_article(nick: &str, slug: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Blog/Article/{slug}")
    }

    pub fn user_wall(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Wall")
    }

    pub fn user_souk(nick: &str) -> String {
        format!("{USERS_PREFIX}{nick}/Souk")
    }

    /// Shortest path of a tab of a member's Souk: `…/Souk`, `…/Souk/Offers` and
    /// `…/Souk/Offers/Perfumes` are the same page (offers of bottles), as are
    /// `…/Souk/Requests` and `…/Souk/Requests/Perfumes`.
    pub fn user_souk_tab(path: &str) -> String {
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix("/Perfumes").unwrap_or(path);
        path.strip_suffix("/Souk/Offers").map(|p| format!("{p}/Souk")).unwrap_or_else(|| path.to_string())
    }

    /// `segment`: `Item` (offer or request) or `Sharing` (split).
    pub fn souk_item(nick: &str, segment: &str, id: u64) -> String {
        format!("{USERS_PREFIX}{nick}/Souk/{segment}/{id}")
    }

    pub fn souk_latest(kind: crate::models::SoukKind) -> &'static str {
        use crate::models::SoukKind;
        match kind {
            SoukKind::Offers => SOUK_OFFERS,
            SoukKind::Requests => SOUK_REQUESTS,
            SoukKind::Sharings => SOUK_SHARINGS,
        }
    }

    /// Value of the `s_type` search field.
    pub fn souk_search_type(kind: crate::models::SoukKind) -> &'static str {
        use crate::models::SoukKind;
        match kind {
            SoukKind::Offers => "offer",
            SoukKind::Requests => "need",
            SoukKind::Sharings => "sharing",
        }
    }

    /// Path prefixes `fetch_page` refuses (internal interfaces, account deletion,
    /// password / e-mail changes, mobile API).
    pub const FETCH_FORBIDDEN_PREFIXES: &[&str] = &[
        "/api/",
        "/api",
        "/action/",
        "/action_fdb/",
        "/app_v5/",
        "/account/delete",
        "/account/password",
        "/account/email",
        "/account/otl",
        "/account_create.php",
        "/login.php",
        "/cdn-cgi/",
    ];

    /// Native page sizes (items per page) of each listing.
    pub const PAGE_SIZE_DEFAULT: u32 = 20;
    pub const PAGE_SIZE_TOPS: u32 = 100;
    /// The site never shows more than this many results of a listing.
    pub const MAX_LISTED_RESULTS: u32 = 1000;
    /// Largest `n` requested from the review / statement endpoints in one call.
    pub const MAX_SLICE: u32 = 50;

    pub fn with_page(path: &str, page: u32) -> String {
        if page <= 1 {
            path.to_string()
        } else if path.contains('?') {
            format!("{path}&current_page={page}")
        } else {
            format!("{path}?current_page={page}")
        }
    }

    pub fn perfume(id: &str) -> String {
        format!("{PERFUMES_PREFIX}{id}")
    }

    pub fn brand(slug: &str) -> String {
        format!("{PERFUMES_PREFIX}{slug}")
    }

    pub fn perfumer(slug: &str) -> String {
        format!("{PERFUMERS_PREFIX}{slug}")
    }

    pub fn note(slug: &str, category: bool) -> String {
        if category { format!("{NOTE_CATEGORY_PREFIX}{slug}") } else { format!("{NOTE_PREFIX}{slug}") }
    }

    pub fn ranking(kind: &RankingKind) -> String {
        match kind {
            RankingKind::TopMen => "/Perfumes/Tops/Men".into(),
            RankingKind::TopWomen => "/Perfumes/Tops/Women".into(),
            RankingKind::TopUnisex => "/Perfumes/Tops/Unisex".into(),
            RankingKind::New => "/Recently_added".into(),
            RankingKind::Trends | RankingKind::TrendsWished | RankingKind::TrendsWorn => "/Perfumes/Trends".into(),
            RankingKind::Year(y) => format!("/Release_Years/{y}"),
        }
    }

    /// Index of the section of `/Perfumes/Trends` a trend ranking reads.
    pub fn trends_section(kind: &RankingKind) -> Option<usize> {
        match kind {
            RankingKind::Trends => Some(0),
            RankingKind::TrendsWished => Some(1),
            RankingKind::TrendsWorn => Some(2),
            _ => None,
        }
    }

    pub fn review_sort(sort: ReviewSort) -> &'static str {
        match sort {
            ReviewSort::Relevance => "order_relevance_desc",
            ReviewSort::Newest => "order_date_desc",
            ReviewSort::Oldest => "order_date_asc",
            ReviewSort::ScentDesc => "order_scent_desc",
            ReviewSort::ScentAsc => "order_scent_asc",
            ReviewSort::MostReplies => "order_replies_desc",
        }
    }

    pub fn search_sort(sort: SearchSort) -> Option<&'static str> {
        Some(match sort {
            SearchSort::Default => return None,
            SearchSort::NameAsc => "n_asc",
            SearchSort::NameDesc => "n_desc",
            SearchSort::Newest => "p_id_desc",
            SearchSort::PopularityDesc => "nr_desc",
            SearchSort::PopularityAsc => "nr_asc",
            SearchSort::YearDesc => "y_desc",
            SearchSort::YearAsc => "y_asc",
            SearchSort::ScentDesc => "v_desc",
            SearchSort::DurabilityDesc => "d_desc",
            SearchSort::SillageDesc => "s_desc",
            SearchSort::BottleDesc => "b_desc",
            SearchSort::PricingDesc => "p_desc",
        })
    }

    // -----------------------------------------------------------------------
    // Forum and communities, read only (DISCOVERY §2.1; measured at jalon 4)
    // -----------------------------------------------------------------------
    /// Forum home: the categories, grouped in sections.
    pub const FORUMS: &str = "/forums";
    pub const FORUM_PREFIX: &str = "/forums/";
    pub const FORUM_TOPIC_PREFIX: &str = "/forums/topic/";
    /// `/forums/post/{id}` answers `200` with the thread page that holds the post.
    pub const FORUM_POST_PREFIX: &str = "/forums/post/";
    /// First path segments under `/forums/` that are not category slugs.
    pub const FORUM_RESERVED: &[&str] = &["topic", "post"];
    /// Forum search. The text goes in `filter` (a `q` parameter is ignored: measured).
    pub const FORUM_SEARCH: &str = "/s_forum_x.php";
    pub const FORUM_SEARCH_TEXT_PARAM: &str = "filter";
    pub const FORUM_SEARCH_PAGE_PARAM: &str = "current_page";
    /// Longest search text the site's form accepts.
    pub const FORUM_SEARCH_MAX_QUERY: usize = 160;
    pub const FORUM_TOPICS_PER_PAGE: u32 = 50;
    pub const FORUM_POSTS_PER_PAGE: u32 = 25;
    /// Directory of communities (`/communities` itself is the feed of all posts).
    pub const COMMUNITIES_INDEX: &str = "/communities/discover";
    pub const COMMUNITY_PREFIX: &str = "/communities/";
    pub const COMMUNITY_POST_SEGMENT: &str = "post";
    /// First path segments under `/communities/` that are not community slugs.
    pub const COMMUNITY_RESERVED: &[&str] = &["all", "discover", "search", "community"];
    pub const COMMUNITIES_PER_PAGE: u32 = 12;
    pub const COMMUNITY_POSTS_PER_PAGE: u32 = 20;
    /// Upper bound of directory pages read by one `communities()` call.
    pub const COMMUNITIES_MAX_PAGES: u32 = 30;

    /// Pages of a category are path segments: `/forums/{slug}/{page}`.
    pub fn forum_category(slug: &str, page: u32) -> String {
        if page <= 1 { format!("{FORUM_PREFIX}{slug}") } else { format!("{FORUM_PREFIX}{slug}/{page}") }
    }

    /// Pages of a topic are path segments: `/forums/topic/{slug}/{page}`.
    pub fn forum_topic(slug: &str, page: u32) -> String {
        if page <= 1 { format!("{FORUM_TOPIC_PREFIX}{slug}") } else { format!("{FORUM_TOPIC_PREFIX}{slug}/{page}") }
    }

    pub fn forum_post(id: u64) -> String {
        format!("{FORUM_POST_PREFIX}{id}")
    }

    /// The directory and the feed of a community paginate with `?page=N`.
    pub fn communities_index(page: u32) -> String {
        if page <= 1 { COMMUNITIES_INDEX.to_string() } else { format!("{COMMUNITIES_INDEX}?page={page}") }
    }

    pub fn community(slug: &str, page: u32) -> String {
        if page <= 1 { format!("{COMMUNITY_PREFIX}{slug}") } else { format!("{COMMUNITY_PREFIX}{slug}?page={page}") }
    }

    pub fn community_post(group: &str, post: &str) -> String {
        format!("{COMMUNITY_PREFIX}{group}/{COMMUNITY_POST_SEGMENT}/{post}")
    }
}

/// CSS selectors.
pub mod sel {
    // --- generic page ---
    pub const TITLE: &str = "title";
    pub const CANONICAL: &str = "link[rel=canonical]";
    pub const OG_IMAGE: &str = "meta[property='og:image']";

    // --- perfume page ---
    pub const P_H1: &str = "h1.p_name_h1";
    pub const P_BRAND_LINK: &str = "h1.p_name_h1 [itemprop=brand] a[itemprop=url]";
    pub const P_YEAR_LINK: &str = "h1.p_name_h1 a[href*='/Release_Years/']";
    pub const P_CONCENTRATION: &str = "h1.p_name_h1 .p_con";
    pub const P_GENDER_ICON: &str = ".p_gender_big i";
    pub const P_RANK_LABEL: &str = ".p_details_section_second a[href*='/Perfumes/Tops/']";
    pub const P_DESCRIPTION: &str = "[itemprop=description]";
    /// Children of the description that are buttons, not text.
    pub const P_DESCRIPTION_SKIP_CLASS: &str = "pbtn-link";
    pub const P_ACCORDS: &str = ".s-circle-container > .text-xs";
    pub const P_NOTES_TOP: &str = ".notes_list .nb_t .clickable_note_img";
    pub const P_NOTES_HEART: &str = ".notes_list .nb_m .clickable_note_img";
    pub const P_NOTES_BASE: &str = ".notes_list .nb_b .clickable_note_img";
    pub const P_NOTES_FLAT: &str = ".notes_list .nb_n .clickable_note_img";
    pub const P_PERFUMERS: &str = "main a[href*='/Perfumers/']";
    pub const P_RATING_ITEM: &str = ".rating-item[data-type]";
    pub const P_RATING_SCORE: &str = ".rating-score";
    pub const P_RATING_COUNT: &str = ".rating-count";
    pub const P_FACTS: &str = ".p_box_infos .leading-7";

    // --- classification chart fragment ---
    pub const CLASS_COL: &str = "#classification_community .col";
    pub const CLASS_TITLE: &str = ".pchart-title";
    pub const CLASS_SVG: &str = "svg.pchart[aria-label]";
    pub const CLASS_VOTES: &str = ".pchart-box-votes";
    /// Placeholder shown when nobody classified the perfume yet.
    pub const CLASS_EMPTY: &str = ".fa-pie-chart";

    // --- reviews ---
    pub const REVIEW: &str = "article.review";
    pub const REVIEW_HEADER: &str = ".review_header[id]";
    pub const REVIEW_DATE: &str = "[itemprop=datePublished]";
    pub const REVIEW_LINK: &str = ".review_header a[href*='/reviews/']";
    pub const REVIEW_AUTHOR: &str = "[itemprop=author] [itemprop=name]";
    pub const REVIEW_AUTHOR_LINK: &str = "a.review_user_photo";
    pub const REVIEW_TITLE: &str = "[id^='r_title_']";
    pub const REVIEW_TEXT: &str = "[id^='r_text_']";
    /// Present inside the text when only an excerpt is shown.
    pub const REVIEW_TRUNCATED: &str = ".review-hover";
    pub const REVIEW_AWARDS: &str = "[id^='nr_awards_']";
    pub const REVIEW_COMMENTS: &str = "[class^='comments_nr_reviews_']";
    pub const VOTING_ITEM: &str = ".voting-icon-nr";
    pub const VOTING_NR: &str = ".nr";

    // --- statements ---
    pub const STATEMENT: &str = "div.statement[id^='tablerow']";
    pub const STATEMENT_AUTHOR_LINK: &str = ".statement-top-left a[href*='/Users/']";
    pub const STATEMENT_DATE: &str = ".statement-top-left .text-xs";
    pub const STATEMENT_LINK: &str = ".statement-top-right a[href*='/statements/']";
    pub const STATEMENT_TEXT: &str = "[id^='s_text_content_']";
    pub const STATEMENT_AWARDS: &str = "[id^='nr_awards_']";
    pub const STATEMENT_COMMENTS: &str = "[class^='comments_nr_statements_']";

    // --- similar perfumes ---
    pub const SIM_ITEM: &str = ".sim_item[data-s_id]";
    pub const SIM_IMG: &str = "img";
    pub const SIM_FILL: &str = ".fill[data-percentage]";
    pub const SIM_NAME: &str = "div.text-xs";
    pub const SIM_CONCENTRATION: &str = "div.text-xs span";
    /// Present in the `get_inspiration` fragment even when there is no similar perfume.
    pub const SIM_HOLDER: &str = "#sim_holder, .action_similar, .action_similar_my";
    pub const PEEK_LINK: &str = ".title a[href*='/Perfumes/']";
    pub const PEEK_VOTES: &str = ".total_all";

    // --- dupes directory ---
    pub const DUPE_ITEM: &str = ".dupe_item";
    pub const DUPE_MAIN_LINK: &str = ".dupe_main a[href*='/Perfumes/']";
    pub const DUPE_MAIN_IMG: &str = ".dupe_main img";
    pub const DUPE_MAIN_NAME: &str = ".dupe_main a span.flex > div:last-child";
    pub const DUPE_MAIN_BRAND: &str = "span.text-sm";
    pub const DUPE_DUPE: &str = ".dupe_dupe";
    pub const DUPE_DUPE_LINK: &str = "a[href*='/Perfumes/']";
    pub const DUPE_DUPE_NAME: &str = "a > span.bold";
    pub const DUPE_DUPE_BRAND: &str = "a span.lightblue";
    pub const DUPE_DUPE_VOTES: &str = ".dupe_dupe > span.lightgrey";
    pub const LABEL_A: &str = ".label_a";

    // --- listings (search, brand, perfumer, note, rankings) ---
    pub const GRID: &str = ".pgrid";
    pub const GRID_COL: &str = ".pgrid > .col";
    pub const COL: &str = ".col";
    pub const COL_LINK: &str = ".name a[href*='/Perfumes/']";
    pub const COL_IMG: &str = ".image img";
    pub const COL_BRAND: &str = ".brand a";
    pub const COL_YEAR: &str = ".release_year";
    pub const COL_GENDER_ICON: &str = ".name i.fa";
    pub const COL_RATING: &str = ".av_scent";
    pub const COL_RATING_COUNT: &str = ".av .tv";
    pub const COL_TREND_COUNT: &str = ".col > .tv";
    pub const COL_REVIEWS: &str = ".nr_reviews i[title]";
    pub const COL_PLACE: &str = ".place";
    pub const PAGING_COUNT: &str = ".paging_nrs_cnt";
    pub const PAGE_LINKS: &str = "a[href*='current_page=']";
    /// Shown instead of the grid when a search has no result.
    pub const NOTHING_FOUND: &str = ".nothing_info_img";
    pub const SECTION_TITLE: &str = "h2";

    // --- brand / perfumer / note heads ---
    pub const ENTITY_H1: &str = ".entity-head h1, .main h1, main h1, h1";
    pub const ENTITY_H1_LINK: &str = "a";
    pub const ENTITY_TAGS: &str = ".entity-head-tags .label_a";
    pub const ENTITY_FLAG: &str = "img.flag";
    pub const ENTITY_SORT_FORM: &str = "form.sort-row[action]";
    pub const NOTE_CATEGORY_LINKS: &str = "a[href*='/Fragrance_Notes/']";

    // --- autocompletion fragments ---
    pub const LABEL_FILTER_CELL: &str = "td[data-name]";
    pub const LIVESEARCH_ITEM: &str = ".ls-perfume-item";
    pub const LIVESEARCH_ITEM_NAME: &str = ".bold";
    pub const LIVESEARCH_ITEM_LINK: &str = "a.ls-perfume-overlay[href]";
    pub const LIVESEARCH_NOTE: &str = "[data-name_url]";

    // --- generic text extraction (fetch_page) ---
    pub const MAIN_CANDIDATES: &[&str] = &["main", ".main", ".content_wrapper", ".page-wrapper", "body"];
    pub const TEXT_SKIP_TAGS: &[&str] = &[
        "script", "style", "noscript", "svg", "nav", "header", "footer", "form", "template", "iframe", "button", "select", "option", "input",
        "textarea",
    ];
    pub const TEXT_BLOCK_TAGS: &[&str] = &[
        "p",
        "div",
        "br",
        "li",
        "ul",
        "ol",
        "tr",
        "table",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "article",
        "section",
        "blockquote",
        "td",
        "th",
        "hr",
    ];
    pub const TEXT_SKIP_IDS: &[&str] = &["login-modal-content", "disable_div"];
    pub const LINKS: &str = "a[href]";

    // --- ShelfControl (own collection) ---
    pub const SHELF_WRAP: &str = ".psheet-wrap[data-lists]";
    pub const SHELF_DATA: &str = "script#psheet-data";
    /// Header cells of custom columns: `data-col="c{id}"`, text = column name.
    pub const SHELF_CUSTOM_COL: &str = "th[data-col^='c']";

    // --- wear tracker (page and calendar fragment) ---
    pub const WEAR_ROW: &str = ".col[id^='tablerow']";
    pub const WEAR_LINK: &str = ".name a[href*='/Perfumes/']";
    pub const WEAR_IMG: &str = ".image img";
    pub const WEAR_WHEN: &str = ".action_tracker_details[data-l_id]";
    pub const WEAR_NOTE_ICON: &str = ".fa-sticky-note[title]";
    /// Root of the calendar fragment, present even for a month without wears.
    pub const WEAR_CALENDAR: &str = ".tcal[data-y][data-m]";
    /// Tabs of the tracker page, present even without any wear.
    pub const WEAR_TRACKER_TABS: &str = "a[href*='/assistant/tracker_calendar']";

    // --- logged-in page header ---
    /// Nickname of the logged-in member, in the header of every page.
    pub const NICK_NAME: &str = ".nick_name";

    // --- rating panel and "My Ratings" page ---
    /// One star widget per scale: class `stars_{scale}`, value in stars out of 5.
    pub const RATE_STARS: &str = ".pstars[data-pstars-value]";
    /// Remove link of a scale: class `vote_remove_{p_id}_{scale}`, `hide` when not rated.
    pub const RATE_REMOVE: &str = "a[class*='vote_remove_']";
    pub const RATINGS_ITEM: &str = ".user-ratings-item[id^='tablerow']";
    pub const RATINGS_LINK: &str = ".name a[href*='/Perfumes/']";
    pub const RATINGS_IMG: &str = ".image img";
    pub const RATINGS_AGE: &str = ".time-ago";
    /// Tabs of the page, present even when nothing is rated.
    pub const RATINGS_TABS: &str = "a[href*='/Ratings/']";

    // --- classification panel ---
    pub const CLASSI_GROUP: &str = ".classification_group";
    pub const CLASSI_GROUP_TITLE: &str = ".bold";
    pub const CLASSI_OPTION: &str = ".action_add_classi[data-c][data-ct]";
    /// Class of an option the member voted for.
    pub const CLASSI_VOTED_CLASS: &str = "pbtn-green";

    // --- review and statement forms ---
    pub const FORM_PERFUME_ID: &str = "#form_perfume_id";
    pub const REVIEW_DELETE_BTN: &str = ".action_delete_review[data-id]";
    pub const STATEMENT_DELETE_BTN: &str = ".action_delete_statement[data-id]";

    // --- member profile (jalon 4) ---
    pub const PROFILE_HEAD: &str = ".id-card-header h1";
    pub const PROFILE_GENDER_ICON: &str = ".id-card-header h1 i.fa";
    pub const PROFILE_BADGE: &str = ".user-specs .user-badge";
    pub const PROFILE_BADGE_FLAG: &str = "img.flag";
    pub const PROFILE_BADGE_ACTIVITY: &str = "i.fa-clock-o, i.fa-circle";
    pub const PROFILE_SCORE: &str = ".score_idcard";
    pub const PROFILE_POINTS: &str = ".score_idcard a[href*='/Community/Top']";
    pub const PROFILE_PIC_BOX: &str = ".user-pic-box-idcard";
    pub const PROFILE_TAB: &str = ".idcard-nav a";
    pub const PROFILE_TAB_COUNT: &str = ".ptab-cnt";
    pub const PROFILE_ACHIEVEMENT: &str = ".idcard-achievements > *, .idcard-achievements-full > *";
    pub const PROFILE_ACHIEVEMENT_COUNT: &str = "span";
    /// "Subscribe & Follow" button of another member's profile, seen logged in.
    pub const PROFILE_SUBSCRIBE: &str = ".action_apply_content_subscription[data-type='users'][data-item-id]";
    /// Holder of the wall comments: its id ends with the numeric member id.
    pub const PROFILE_WALL_HOLDER: &str = "[id^='comments-id_cards-']";
    pub const PROFILE_WEARING: &str = "#i_wear_sidebar .col .name a[href*='/Perfumes/']";
    pub const PROFILE_SIGNATURE: &str = "#sigscent_sidebar .col .name a[href*='/Perfumes/']";
    pub const PROFILE_LIST: &str = ".wardrobe_list_sidebar a[href*='/Collection/']";
    pub const PROFILE_LIST_COUNT: &str = "span.text-3xs";
    /// Signature scent fragment answered by `account/signature_scent`.
    pub const SIGNATURE_FRAGMENT_LINK: &str = ".col .name a[href*='/Perfumes/']";
    pub const SIGNATURE_FRAGMENT_MARK: &str = ".sig-sel";

    // --- another member's collection list ---
    pub const WARDROBE_WRAPPER: &str = "#wr_wrapper";
    pub const WARDROBE_ITEM: &str = ".war_it[data-id][data-wr-id]";
    pub const WARDROBE_PEEK_LINK: &str = ".title a[href*='/Perfumes/']";
    pub const WARDROBE_PEEK_BRAND: &str = ".title a.lightgrey";

    // --- mailbox (Postbox) ---
    pub const POSTBOX_VIEW: &str = "#pbox-view-container";
    pub const POSTBOX_CONVERSATION: &str = "a[href*='/Postbox/R/']";
    pub const POSTBOX_CONVERSATION_DATE: &str = ".pm_list_item_user_date[data-time]";
    pub const POSTBOX_CONVERSATION_TEXT: &str = ".pm_list_item_message";
    pub const POSTBOX_CONVERSATION_SENT: &str = ".fa-paper-plane-o";
    pub const POSTBOX_TAB_COUNT: &str = ".ptabs a .ptab-cnt";
    pub const POSTBOX_THREAD: &str = "#messages[data-user]";
    pub const POSTBOX_THREAD_NICK: &str = ".pb2_name";
    pub const POSTBOX_MESSAGE: &str = "tr.msg-row-id[data-id]";
    pub const POSTBOX_MESSAGE_TEXT: &str = ".postbox_message_text";
    pub const POSTBOX_MESSAGE_SENT_CLASS: &str = "pm2_sent";

    // --- notifications ---
    pub const NOTIFICATION_ITEM: &str = ".notification-item[data-id]";
    pub const NOTIFICATION_EMPTY: &str = ".notification_history_empty";
    /// Children of an item that are controls, not text.
    pub const NOTIFICATION_SKIP_CLASS: &str = "pm-close";

    // --- follows ---
    pub const FOLLOW_TAB_COUNT: &str = "a[href$='/account/follow/list'] .ptab-cnt";
    pub const FOLLOW_MEMBER: &str = ".user-rows .user-row a[href*='/Users/'], .user-strip a[href*='/Users/']";
    /// Shown instead of the list when nobody is followed ("No Users found").
    pub const FOLLOW_EMPTY: &str = ".main .nothing_info_img";
    /// Tabs of the "I follow" pages, present even when nobody is followed.
    pub const FOLLOW_TABS: &str = "a[href$='/account/follow/list']";

    // --- settings pages ---
    pub const SETTING_TOGGLE: &str = "input.pd-yesno[data-field]";
    pub const SETTING_SELECT: &str = "select.pd-select[data-field]";
    pub const SETTING_OPTION: &str = "option[data-value]";
    pub const SETTING_ROW: &str = "tr";
    pub const SETTING_LABEL: &str = "td.bold, td .bold";
    pub const SETTING_SECTION: &str = "h2";
    pub const PREFERENCES_LIKE: &str = "form[action*='fragrance_preferences'] input[name='i_like']";
    pub const PREFERENCES_DISLIKE: &str = "form[action*='fragrance_preferences'] input[name='i_dont_like']";

    // --- write forms ---
    pub const FORUM_COMPOSE_FORM: &str = "form[data-endpoint*='create_topic'] input[name='forum_id']";
    pub const SOUK_FORM: &str = "form.sk-panel-form";
    pub const SOUK_FORM_FIELD: &str = "input[name], textarea[name]";

    // --- a member's reviews and statements pages ---
    pub const USER_REVIEW: &str = "[class*='review_box_full_']";
    pub const USER_REVIEW_LINK: &str = ".header_right a[href*='/reviews/']";
    pub const USER_REVIEW_AGE: &str = ".header_left .text-xs";
    pub const USER_REVIEW_AWARDS: &str = ".header_left .fr";
    pub const USER_TEXT_PERFUME: &str = "a.bold[href*='/Perfumes/']";
    pub const USER_TEXT_BRAND: &str = "a.lightblue[href*='/Perfumes/']";
    pub const USER_STATEMENT_PERFUME: &str = "a.title[href*='/Perfumes/']";
    /// Heading of a member page (`<h1>nick</h1>`), present even without any text.
    pub const USER_PAGE_HEAD: &str = ".main h1";

    // --- blogs (site-wide feeds, a member's blog, one article) — jalon 4 ---
    /// Tabs of the site-wide feeds, present on every feed page.
    pub const BLOG_FEED_TABS: &str = ".main a[href*='/blogs/top']";
    pub const BLOG_CARD: &str = ".p-box";
    pub const BLOG_CARD_LINK: &str = "a.block[href*='/Blog/Article/']";
    pub const BLOG_CARD_TITLE: &str = "a.block .bold";
    pub const BLOG_CARD_EXCERPT: &str = "a.block .text-sm";
    pub const BLOG_CARD_AUTHOR: &str = ".fl a[href*='/Users/']";
    pub const BLOG_CARD_DATE: &str = ".fl .text-xs";
    pub const BLOG_CARD_COVER: &str = "a.box-media-top img";
    /// Icons whose parent holds the award and comment counters of a card.
    pub const BLOG_CARD_AWARDS_ICON: &str = ".fr .fa-trophy";
    pub const BLOG_CARD_COMMENTS_ICON: &str = ".fr .fa-comments";
    pub const BLOG_ARTICLE: &str = "article.blog_item[id^='tablerow']";
    pub const BLOG_ARTICLE_TITLE: &str = "h1";
    pub const BLOG_ARTICLE_TEXT: &str = "[class*='blog_item_text']";
    pub const BLOG_ARTICLE_IMAGES: &str = "[class*='blog_item_text'] img[src]";
    /// Links to the author's blog in the article head; the one with text is the blog title.
    pub const BLOG_ARTICLE_BLOG_LINK: &str = "a[href$='/Blog']";
    pub const BLOG_ARTICLE_DATE: &str = ".lightblue .text-xs";
    pub const BLOG_ARTICLE_AWARDS: &str = "[id^='nr_awards_']";
    pub const BLOG_ARTICLE_COMMENTS: &str = "[class^='comments_nr_blogs_']";

    // --- comments (wall entries, blog comments) — jalon 4 ---
    pub const COMMENT: &str = ".comment_view_holder[data-comment_id]";
    pub const COMMENT_REPLY: &str = ".comment_view_holder_reply[id^='tablerow']";
    pub const COMMENT_AUTHOR: &str = ".fl a[href*='/Users/']";
    pub const COMMENT_DATE: &str = ".fl .text-xs";
    pub const COMMENT_TEXT: &str = "[class*='comment-text-']";
    /// Link to the profile in the navigation of a member page.
    pub const MEMBER_NAV_PROFILE: &str = ".idcard-nav a[href*='/Users/']";

    // --- Souk (lists, search, a member's Souk, one entry) — jalon 4 ---
    /// Search form, present on every site-wide Souk page, even without result.
    pub const SOUK_SEARCH_FORM: &str = ".souk-search form[action*='s_souk']";
    pub const SOUK_CARD: &str = ".soukgrid .col";
    pub const SOUK_CARD_LINK: &str = "a.souk-card[href]";
    pub const SOUK_CARD_DATE: &str = ".date";
    pub const SOUK_CARD_FLAG: &str = "img.flag_s";
    pub const SOUK_CARD_NAME: &str = ".name";
    pub const SOUK_CARD_CONCENTRATION: &str = ".name .label_a";
    pub const SOUK_CARD_BRAND: &str = ".name .lightblue2, .name .lightblue";
    /// Labels above the picture: `Bottle` / `Sample`, or `16 ml left` and the participants of a split.
    pub const SOUK_CARD_LABEL: &str = ".mb-1 .label_a";
    pub const SOUK_CARD_LABEL_LEFT: &str = ".fa-tint";
    pub const SOUK_CARD_LABEL_PEOPLE: &str = ".fa-user";
    pub const SOUK_CARD_PHOTO: &str = ".fa-camera";
    pub const SOUK_CARD_REQUEST_BADGE: &str = ".souk-badge-need";
    pub const SOUK_CARD_OFFER_BADGE: &str = ".souk-badge-offer";
    pub const SOUK_CARD_REQUEST_CLASS: &str = "souk-need";
    pub const SOUK_CARD_OFFER_CLASS: &str = "souk-offer";
    /// Tabs of a member's Souk, present even when it is empty.
    pub const USER_SOUK_TABS: &str = ".ptabs a[href*='/Souk/']";
    pub const TAB_COUNT: &str = ".ptab-cnt";
    pub const SOUK_HEAD: &str = ".souk-head";
    pub const SOUK_HEAD_PERFUME: &str = ".souk-head-name a[href*='/Perfumes/']";
    pub const SOUK_HEAD_BRAND: &str = ".souk-head-sub a[href*='/Perfumes/']";
    pub const SOUK_HEAD_MEMBER: &str = ".souk-head-meta a[href*='/Users/']";
    pub const SOUK_HEAD_META: &str = ".souk-head-meta > span";
    pub const SOUK_HEAD_TYPE: &str = ".souk-head-type";
    pub const SOUK_HEAD_SECOND: &str = ".souk-head-side .text-sm";
    pub const SOUK_ROW: &str = ".souk-table tr";
    pub const SOUK_ROW_CELL: &str = "td";
    pub const SOUK_RING_VALUE: &str = ".souk-ring-value";
    pub const SOUK_RING_LABEL: &str = ".souk-ring-label";
    pub const SOUK_RING_CAPTION: &str = ".souk-ring-caption";
    pub const SOUK_PHOTO: &str = ".igrid .image a[href]";
    /// Country label of the member in the head of an entry page.
    pub const SOUK_COUNTRY: &str = ".label_a img.flag";
    // -----------------------------------------------------------------------
    // Forum and communities (jalon 4)
    // -----------------------------------------------------------------------
    /// Footer of every page: its absence means the body was cut short.
    pub const PAGE_FOOTER: &str = "#footer_holder";
    /// Wrapper of the forum home (and of category pages), present whatever the rows.
    pub const FORUM_HOME: &str = ".forum_overview";
    /// Section titles and category rows of the forum home, in document order.
    pub const FORUM_HOME_ENTRIES: &str = ".forum_overview .left_wrapper h2, .forum_overview .forum-row";
    pub const FORUM_ROW_CLASS: &str = "forum-row";
    pub const FORUM_ROW_LINK: &str = "a.forum-row-link[href]";
    pub const FORUM_ROW_DESC: &str = ".flex-1 .text-xs .hide-mobile";
    /// "857 topics" / "2 today".
    pub const FORUM_ROW_STATS: &str = ".forum-row-stats";
    /// Category icon: `…/forum/svg/en/f_{forum_id}.svg`.
    pub const FORUM_ROW_ICON: &str = ".forum-row-icon img[src]";
    /// `<script type="application/json" id="forum_context">` of category and topic pages:
    /// `{"forum_id":7,"topic_id":29437,…}`. Present even on an empty page.
    pub const FORUM_CONTEXT: &str = "script#forum_context";
    pub const FORUM_CATEGORY_H1: &str = ".forum_overview h1";
    pub const FORUM_CATEGORY_DESC: &str = ".forum_overview h1 > div";
    pub const TOPIC_ROW: &str = ".topic-row";
    pub const TOPIC_PINNED_ROW: &str = ".topic-pinned .topic-row";
    pub const TOPIC_META: &str = ".topic-row-meta";
    pub const TOPIC_AUTHOR: &str = ".topic-row-meta a[href*='/Users/']";
    /// "Page N" shortcut to the last page of a long topic.
    pub const TOPIC_LAST_PAGE: &str = ".topic-row-meta .label_a > a[href*='/forums/topic/']";
    pub const TOPIC_LOCKED: &str = ".fa-lock";
    pub const TOPIC_LAST_POST: &str = ".topic-row-last a[href*='/forums/post/']";
    pub const TOPIC_LAST_AUTHOR: &str = ".topic-row-last a[href*='/Users/']";
    /// Page numbers of lists whose pages are not `current_page=` links (forum, communities).
    pub const PAGING_NUMBERS: &str = ".paging .numbers a";
    pub const PAGING_ACTIVE: &str = ".paging .numbers .active a";
    pub const THREAD_H1: &str = "h1[itemprop=headline]";
    pub const THREAD_CATEGORY: &str = ".main_full .text-m a[href*='/forums/']";
    /// Wrapper of the posts of a topic page.
    pub const THREAD_POSTS: &str = ".forum-posts";
    pub const POST: &str = ".forum-post[id]";
    pub const POST_AUTHOR: &str = ".forum-post-info [itemprop=name]";
    pub const POST_AUTHOR_LINK: &str = ".forum-post-info a[itemprop=url]";
    pub const POST_DATE: &str = "[itemprop=datePublished]";
    pub const POST_TITLE: &str = ".forum-post-text-title .title";
    pub const POST_TEXT: &str = "[itemprop=text]";
    /// Heart counter of a post ("thanks").
    pub const POST_THANKS: &str = ".forum-post-text-title [data-forum-thanks-count] [itemprop=userInteractionCount]";
    /// Search form of `/s_forum_x.php`, present with or without results.
    pub const FORUM_SEARCH_FORM: &str = "form#s_form[action*='s_forum_x']";
    pub const SEARCH_HIT_AUTHOR: &str = ".topic-row-meta a[href*='/Users/']";
    pub const SEARCH_HIT_CATEGORY: &str = ".topic-row-meta a[href*='/forums/']";
    pub const SEARCH_HIT_EXCERPT: &str = ".topic-row-excerpt";

    // -----------------------------------------------------------------------
    // Communities (jalon 4)
    // -----------------------------------------------------------------------
    /// "105 communities" counter of the directory, present whatever the cards.
    pub const COMMUNITIES_COUNT: &str = "#discover .g-results-count";
    pub const COMMUNITY_CARD: &str = "a.g-card[href]";
    pub const COMMUNITY_CARD_TITLE: &str = ".g-card-title";
    pub const COMMUNITY_CARD_TAGLINE: &str = ".g-card-tagline";
    pub const COMMUNITY_CARD_DESC: &str = ".g-card-desc";
    pub const COMMUNITY_CARD_STATS: &str = ".g-card-stats span";
    /// `Public` / `Invite-only` badge.
    pub const COMMUNITY_BADGE: &str = ".g-badge";
    /// Holder of the posts of a community page, present even without posts.
    pub const COMMUNITY_FEED: &str = "#g_feed_items";
    pub const COMMUNITY_TITLE: &str = ".g-banner-title";
    pub const COMMUNITY_TAGLINE: &str = ".g-banner-tagline";
    pub const COMMUNITY_MEMBERS: &str = ".g-banner-members-count";
    pub const COMMUNITY_STATS: &str = ".g-banner-stats > span";
    pub const COMMUNITY_DESC: &str = "#g_about_description";
    pub const COMMUNITY_TAGS: &str = ".g-tags .g-tag";
    pub const COMMUNITY_GROUP_ID: &str = "[data-group-id]";
    pub const COMMUNITY_POST: &str = "article.g-post[data-post-url]";
    pub const COMMUNITY_POST_AUTHOR: &str = ".g-feed-head-text a[href*='/Users/']";
    pub const COMMUNITY_POST_DATE: &str = ".g-feed-head-text .text-2xs";
    pub const COMMUNITY_POST_TITLE: &str = ".g-feed-text > .bold";
    pub const COMMUNITY_POST_TEXT: &str = ".g-feed-excerpt";
    pub const COMMUNITY_POST_FLAIR: &str = ".g-flair";
    pub const COMMUNITY_POST_VOTE: &str = ".pvote[data-pvote-id]";
    pub const COMMUNITY_POST_SCORE: &str = ".pvote-score";
    pub const COMMUNITY_BOTTLE: &str = ".g-feed-bottle";
    pub const COMMUNITY_BOTTLE_NAME: &str = ".g-feed-bottle-name";
    /// Root of a single community post page.
    pub const COMMUNITY_POST_DETAIL: &str = "article.g-post-detail";
    pub const COMMUNITY_POST_DETAIL_TITLE: &str = ".g-post-detail-title";
    pub const COMMUNITY_POST_DETAIL_AUTHOR: &str = ".g-post-detail-meta a.g-author";
    pub const COMMUNITY_POST_DETAIL_META: &str = ".g-post-detail-meta > span";
    pub const COMMUNITY_POST_DETAIL_BODY: &str = ".g-post-detail-body";
    /// Link back to the community, in the small banner of a post page.
    pub const COMMUNITY_MINI_LINK: &str = "a.g-banner-mini-link[href]";
}

/// Regular expressions applied to raw HTML or attribute values.
pub mod re {
    /// `getPerfumeReviews('<total>',reviews_o,'Nein',false,'<p_id>','<h>')`
    pub const REVIEWS_CALL: &str = r"getPerfumeReviews\('(\d+)',\s*reviews_o,\s*'[A-Za-z]+',\s*(?:true|false),\s*'(\d+)',\s*'([0-9a-f]{32})'\)";
    /// `getPerfumeStatements('<total>',statements_o,false,'<p_id>','<h>')`
    pub const STATEMENTS_CALL: &str = r"getPerfumeStatements\('(\d+)',\s*statements_o,\s*(?:true|false),\s*'(\d+)',\s*'([0-9a-f]{32})'\)";
    /// `get_classification_chart.php",{type:type,p:<p_id>,h:'<h>',…`
    pub const CLASS_CALL: &str = r"get_classification_chart\.php.{0,40}?p:\s*(\d+),\s*h:\s*'([0-9a-f]{32})'";
    /// Fallback sources of the numeric perfume id.
    pub const P_ID_FALLBACK: &str = r#"(?:data-p_id="|p_id:'|"p_id_1":)(\d+)"#;
    pub const H_TOKEN: &str = r"^[0-9a-f]{32}$";
    /// `1 - 20 by 344`
    pub const PAGING_COUNT: &str = r"(\d[\d.,]*)\s*-\s*(\d[\d.,]*)\s+by\s+(\d[\d.,]*)";
    pub const CURRENT_PAGE: &str = r"[?&;]current_page=(\d+)";
    pub const REVIEW_CLASS_ID: &str = r"review_article_(\d+)";
    pub const TRAILING_ID: &str = r"(\d+)$";
    pub const YEAR: &str = r"\b(1[5-9]\d\d|20\d\d)\b";
    pub const FIRST_INT: &str = r"(\d[\d.,]*)";
    pub const PERCENT_PAIR: &str = r"^\s*(.+?)\s+(\d+(?:\.\d+)?)%\s*$";
    pub const BRAND_ID: &str = r"[?&;]b_id=(\d+)";
    pub const PERFUMER_ID: &str = r"[?&;]pf_id=(\d+)";
    pub const NOTE_ID: &str = r"inc=a:(\d+)";
    pub const SINCE_YEAR: &str = r"(?i)since\s+(\d{4})";
    pub const CSRF_JSON: &str = r#""csrf_key"\s*:\s*"([A-Za-z0-9]{8,64})""#;
    /// `Thursday, 12/11/2026, 07:21 AM` (day/month/year, 12-hour clock).
    pub const WEAR_WHEN: &str = r"(\d{1,2})/(\d{1,2})/(\d{4}),\s*(\d{1,2}):(\d{2})\s*(AM|PM)";
    /// `getWardrobe(6467,'Nick','memo','Watch_List','','','9')` in a member's collection page.
    pub const WARDROBE_CALL: &str = r"getWardrobe\((\d+),\s*'([^']*)',\s*'([^']*)',\s*'([^']*)',\s*'[^']*',\s*'[^']*',\s*'(\d*)'\)";
    pub const WALL_HOLDER_ID: &str = r"comments-id_cards-(\d+)$";
    /// `writeComment('#comment-f-id_cards-{id}-0')`: the "Post to Wall" button of a profile.
    pub const WALL_BUTTON_ID: &str = r"comment-f-id_cards-(\d+)-";
    pub const MEMBER_SINCE: &str = r"Member since\s+(\d{1,2}/\d{1,2}/\d{4})";
    pub const FOLLOWERS: &str = r"([\d.,]+)\s+followers";
    /// `$("#comment-f-id_cards-{id}-0").data("conf",{id:…,comment_id:…,type:"id_cards",…,redirect:…,p1:…,p2:…,np:…,l:…,h:…})`
    /// in a profile page served to a session: what the wall form posts.
    pub const WALL_CONF: &str = r"\.data\(.conf.,\s*\{id:'(\d+)',comment_id:'(\d*)',type:'id_cards'[^}]*?redirect:'([^']*)'[^}]*?p1:'([^']*)',p2:'([^']*)',np:'[^']*',l:'([^']*)',h:'([0-9a-f]{32})'";
    pub const SHELF_CUSTOM_COL_ID: &str = r"^c(\d+)$";
    pub const RATE_SCALE_CLASS: &str = r"stars_(scent|durability|sillage|bottle|pricing)\b";
    pub const RATE_REMOVE_CLASS: &str = r"vote_remove_\d+_(scent|durability|sillage|bottle|pricing)\b";
    pub const USER_REVIEW_CLASS_ID: &str = r"review_box_full_(\d+)";
    // --- blogs, Souk, wall (jalon 4) ---
    /// `/Users/{nick}/Blog/Article/{slug}`
    pub const BLOG_ARTICLE_URL: &str = r"/Users/([^/?#]+)/Blog/Article/([^/?#]+)";
    /// `getAllComments('blogs','<article id>','<author id>',…` in the article page.
    pub const BLOG_AUTHOR_ID: &str = r"getAllComments\('blogs',\s*'\d+',\s*'(\d+)'";
    /// `/Users/{nick}/Souk/Item/{id}` (offer, request) or `/Users/{nick}/Souk/Sharing/{id}` (split).
    pub const SOUK_ITEM_URL: &str = r"/Users/([^/?#]+)/Souk/(Item|Sharing)/(\d+)";
    /// `…user_souk_items_sidebar.php",{u:<member id>,n:'<nick>',…` in a Souk entry page.
    pub const SOUK_MEMBER_ID: &str = r#"sidebar\.php",\s*\{\s*u:\s*(\d+)"#;
    /// `flags_png/de.png`
    pub const FLAG_CODE: &str = r"flags_png/([A-Za-z_-]{2,8})\.png";
    /// `writeComment('#comment-f-id_cards-<member id>-0')`: the "Post to Wall" button.
    pub const WALL_OWNER_ID: &str = r"comment-f-id_cards-(\d+)-";

    // --- forum and communities (jalon 4) ---
    /// Category icon of the forum home: `f_7.svg` → forum id 7.
    pub const FORUM_ICON_ID: &str = r"/f_(\d+)\.svg";
    /// `16,488 replies`
    pub const TOPIC_REPLIES: &str = r"(\d[\d.,]*)\s+repl";
    /// `857 topics`, `2 today`
    pub const COUNT_TOPICS: &str = r"(\d[\d.,]*)\s+topic";
    pub const COUNT_TODAY: &str = r"(\d[\d.,]*)\s+today";
    /// `/forums/post/238772#238772`, `/forums/post/238548?source=forum_search#238548`
    pub const FORUM_POST_ID: &str = r"/forums/post/(\d+)";
    /// `198 members`, `243 posts`, `105 communities`, `2 votes`
    pub const COUNT_MEMBERS: &str = r"(\d[\d.,]*)\s+member";
    pub const COUNT_POSTS: &str = r"(\d[\d.,]*)\s+post";
    pub const COUNT_COMMUNITIES: &str = r"(\d[\d.,]*)\s+communit";
    pub const COUNT_VOTES: &str = r"(-?\d[\d.,]*)\s+vote";
    /// Inline script of a community post page: `var postId  = 3592;`
    pub const COMMUNITY_POST_ID: &str = r"var\s+postId\s*=\s*(\d+)";
    /// JSON-LD of a community post page: number of comments and ISO date.
    pub const COMMUNITY_COMMENT_COUNT: &str = r#"schema\.org/CommentAction","userInteractionCount":(\d+)"#;
    pub const JSONLD_DATE_PUBLISHED: &str = r#""datePublished":"([^"]+)""#;
}

/// Text markers.
pub mod markers {
    /// Separator between perfume name and brand in image `alt` texts: `Sauvage by Dior`.
    pub const ALT_BY: &str = " by ";
    /// Title of the standard 404 page.
    pub const NOT_FOUND_TITLE: &str = "404";
    /// Cloudflare challenge / Turnstile fingerprints (interstitial pages only; the passive
    /// `/cdn-cgi/challenge-platform/scripts/jsd/main.js` beacon of normal pages is not one).
    pub const CHALLENGE: &[&str] =
        &["<title>Just a moment", "cf-chl-", "cf_chl_opt", "challenges.cloudflare.com/turnstile", "cf-turnstile", "Attention Required! | Cloudflare"];
    pub const CHALLENGE_HEADER: &str = "cf-mitigated";
    pub const INVALID_CSRF: &str = "Invalid CSRF token";
    pub const INVALID_LOGIN: &str = "Invalid username";
    /// Cookie carrying the persistent login, measured at jalon 0 bis (DISCOVERY §4): 64 hex
    /// characters, set by a successful login, enough on its own to rebuild a session, revoked
    /// by `/api/account/logout`. (`user_id` / `user_id_key` are legacy names the site only clears.)
    pub const LOGIN_COOKIES: &[&str] = &["pf_auth"];
    pub const SESSION_COOKIE: &str = "PHPSESSID";
    pub const GENDER_MEN: &str = "fa-mars";
    pub const GENDER_WOMEN: &str = "fa-venus";
    pub const GENDER_UNISEX: &str = "fa-venus-mars";
    /// Gender codes of the ShelfControl JSON (German) → value used by this client.
    pub const SHELF_GENDERS: &[(&str, &str)] = &[("maennlich", "men"), ("weiblich", "women"), ("unisex", "unisex")];
    /// Fixed lists: id, slug of the site, accepted aliases (lower case, `_` separated).
    pub const FIXED_LISTS: &[(u64, &str, &[&str])] = &[
        (1, "have", &["i_have", "owned", "collection"]),
        (2, "had", &["i_had"]),
        (3, "want", &["wish_list", "wishlist", "wish"]),
        (4, "memo", &["watch_list", "watchlist", "watch"]),
        (5, "tested", &["tests"]),
        (6, "filling", &["decants", "decant"]),
        (7, "mini", &["miniatures", "miniature", "minis"]),
    ];
    /// Headline of a Souk entry page: `Offering Bottle`, `Looking for Bottle`, `Split`.
    pub const SOUK_HEAD_OFFER: &str = "Offering";
    pub const SOUK_HEAD_REQUEST: &str = "Looking for";
    /// URL segment of a split (`/Souk/Sharing/{id}`); offers and requests use `Item`.
    pub const SOUK_SHARING_SEGMENT: &str = "Sharing";
    pub const SOUK_ITEM_SEGMENT: &str = "Item";
    /// Rows of the table of a Souk entry page (label in lower case contains …) → field.
    pub const SOUK_ROWS: &[(&str, &str)] = &[
        ("souk option", "trade"),
        ("condition", "condition"),
        ("price", "price"),
        ("shipping", "shipping"),
        ("swap", "swap_for"),
        ("exchange", "swap_for"),
    ];
    /// Rating labels shown next to member votes → scale.
    pub const VOTE_LABELS: &[(&str, &str)] =
        &[("scent", "scent"), ("longevity", "durability"), ("sillage", "sillage"), ("bottle", "bottle"), ("value", "pricing"), ("pric", "pricing")];

    // --- forum and communities (jalon 4) ---
    /// Keys of the `forum_context` JSON of category and topic pages.
    pub const FORUM_CONTEXT_FORUM_ID: &str = "forum_id";
    pub const FORUM_CONTEXT_TOPIC_ID: &str = "topic_id";
    pub const COMMUNITY_GROUP_ID_ATTR: &str = "data-group-id";
    pub const COMMUNITY_POST_URL_ATTR: &str = "data-post-url";
    pub const COMMUNITY_POST_ID_ATTR: &str = "data-pvote-id";
    /// Separator between name and brand of a perfume attached to a community post
    /// (the site's own convention in perfume links: `Sauvage | Dior`).
    pub const PERFUME_BRAND_SEP: &str = " | ";
    /// Why a community post page comes without its comments.
    pub const COMMUNITY_COMMENTS_ABSENT: &str =
        "comments are not part of the public page: Parfumo loads them through a call reserved to logged-in members";
}
