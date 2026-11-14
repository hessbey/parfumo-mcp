//! Account and member side of jalon 4: public profiles and collections of other members,
//! mailbox, notifications, follows, settings, profile fields, and the builders of the
//! publicly visible writes (messages, wall, forum, blog, Souk).
//!
//! Measured logged in at jalon 0 bis (DISCOVERY §3.6, §3.7): mailbox, follow, settings,
//! fragrance preferences, signature scent. Only read in the site's forms and scripts,
//! never sent: wall, forum, blog and Souk writes.

use std::collections::BTreeMap;

use crate::{
    client::{LISTING_TTL, ParfumoClient},
    error::ClientError,
    models::*,
    parse,
    site::routes,
    transport::empty_body,
};

/// Builders of the write requests of jalon 4. Nothing is sent here.
pub mod social_calls {
    use super::*;

    fn call(path: &str, expect: Expect, form: &[(&str, String)]) -> WriteCall {
        WriteCall { method: "POST".into(), path: path.to_string(), form: form.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), expect }
    }

    /// Sends a private message. Answer: `1`; anything else is the refusal text.
    pub fn message_send(member_id: u64, text: &str) -> WriteCall {
        call(routes::POSTBOX_SEND, Expect::One, &[("user_id", member_id.to_string()), ("msg", text.into()), ("temp_images", "0".into())])
    }

    /// Follows or unfollows a member (empty answer: only a re-read proves the effect).
    pub fn follow(member_id: u64, add: bool) -> WriteCall {
        call(
            routes::SUBSCRIPTION,
            Expect::Anything,
            &[
                ("operation", if add { "add" } else { "remove" }.into()),
                ("item_id", member_id.to_string()),
                ("type", "users".into()),
                ("sub_type", "follow".into()),
            ],
        )
    }

    /// Changes one setting (empty answer whatever the field: only a re-read proves the effect).
    pub fn setting_set(field: &str, value: &str) -> WriteCall {
        call(routes::SETTING_SET, Expect::Anything, &[("a", field.into()), ("v", value.into())])
    }

    /// Saves the two fragrance preference lines of the profile (answer: redirect to the profile).
    pub fn fragrance_preferences_save(i_like: &str, i_dont_like: &str) -> WriteCall {
        call(
            routes::FRAGRANCE_PREFERENCES_SAVE,
            Expect::Redirect,
            &[("do_edit", "1".into()), ("i_like", i_like.into()), ("i_dont_like", i_dont_like.into())],
        )
    }

    /// Sets (`Some(p_id)`) or removes (`None`) the signature scent of the profile.
    pub fn signature_scent(p_id: Option<u64>) -> WriteCall {
        call(routes::SIGNATURE_SCENT, Expect::Body, &[("p", p_id.map(|p| p.to_string()).unwrap_or_else(|| "clear".into()))])
    }

    /// Writes on a member's wall. `conf` comes from the wall's comment form; its token is
    /// sent, and replaced by a placeholder when the call is only shown. Seen, never sent.
    pub fn wall_post(conf: &CommentConf, text: &str, show_token: bool) -> WriteCall {
        call(
            routes::COMMENT_POST,
            Expect::CommentOk,
            &[
                ("a", conf.kind.clone()),
                ("id", conf.item_id.to_string()),
                ("comment_id", conf.comment_id.clone()),
                ("text", text.into()),
                ("p1", conf.p1.clone()),
                ("p2", conf.p2.clone()),
                ("redirect", conf.redirect.clone()),
                ("l", conf.l.clone()),
                ("h", if show_token { conf.h.clone() } else { "<token of the wall's comment form>".into() }),
            ],
        )
    }

    /// Opens a forum topic. Seen in the form, never sent.
    pub fn forum_topic(forum_id: u64, subject: &str, message: &str) -> WriteCall {
        call(
            routes::FORUM_CREATE_TOPIC,
            Expect::Success,
            &[("forum_id", forum_id.to_string()), ("subject", subject.into()), ("message", message.into())],
        )
    }

    /// Replies in a forum topic. `topic` is the numeric topic id. Seen in the script, never sent.
    pub fn forum_reply(topic_id: u64, message: &str) -> WriteCall {
        call(routes::FORUM_REPLY, Expect::Success, &[("topic_id", topic_id.to_string()), ("message", message.into())])
    }

    /// Submits the blog form ("Save & Preview"). Seen in the form, never sent.
    pub fn blog_post(nick: &str, title: &str, text: &str) -> WriteCall {
        call(
            routes::BLOG_POST,
            Expect::Redirect,
            &[("blog_write_post", "1".into()), ("blog_nick", nick.into()), ("blog_title", title.into()), ("blog_text", text.into())],
        )
    }

    /// Creates (`souk_id` 0) or updates a Souk entry with the fields of its form. Seen, never sent.
    pub fn souk_save(fields: &BTreeMap<String, String>) -> WriteCall {
        let update = fields.get("souk_id").is_some_and(|id| id != "0" && !id.is_empty());
        WriteCall {
            method: "POST".into(),
            path: if update { routes::SOUK_UPDATE } else { routes::SOUK_ADD }.to_string(),
            form: fields.clone(),
            expect: Expect::ResultSuccess,
        }
    }
}

impl ParfumoClient {
    /// Nickname of a member reference: a nickname, a `/Users/{nick}/…` path or a profile URL.
    pub fn member_nick(&self, reference: &str) -> Result<String, ClientError> {
        let hint = || ClientError::InvalidInput("expected a Parfumo nickname or a member profile URL".into());
        let (absolute, path) = self.ref_path(reference)?;
        let rel = if absolute {
            path.strip_prefix(routes::USERS_PREFIX).ok_or_else(hint)?.to_string()
        } else {
            path.strip_prefix("Users/").unwrap_or(&path).to_string()
        };
        let nick = rel.split('/').next().unwrap_or_default().trim().trim_start_matches('@');
        if nick.is_empty() || nick.contains(|c: char| c.is_whitespace() || matches!(c, '?' | '#' | '&' | '%' | '<' | '>' | '"')) {
            return Err(hint());
        }
        Ok(nick.to_string())
    }

    fn member_not_found(e: ClientError, nick: &str) -> ClientError {
        match e {
            ClientError::NotFound(_) => ClientError::NotFound(format!("member '{nick}'")),
            other => other,
        }
    }

    /// Profile of a member. On a logged-in client the page is never cached.
    pub async fn member_profile(&self, reference: &str, cached: bool) -> Result<MemberProfile, ClientError> {
        let nick = self.member_nick(reference)?;
        // Only the public read goes through the application interface. The uncached one is
        // that of the account's own tools, which read what the page alone carries: the
        // signature scent before and after it is changed, the rank.
        if cached
            && self.has_app_session()
            && let Some(answer) = crate::app::settle(self.app_member_profile(&nick).await)
        {
            return answer;
        }
        let path = routes::user(&nick);
        let (_, html) = self.t.get(&path, cached.then_some(LISTING_TTL), !cached, false).await.map_err(|e| Self::member_not_found(e, &nick))?;
        parse::parse_member_profile(&html, &self.t.absolute(&path))
    }

    /// One page of the reviews or statements a member wrote (public pages), newest first.
    pub async fn member_texts_page(&self, reference: &str, kind: TextKind, page: u32) -> Result<Listing<OwnText>, ClientError> {
        let nick = self.member_nick(reference)?;
        let path = match kind {
            TextKind::Review => routes::user_reviews(&nick),
            TextKind::Statement => routes::user_statements(&nick),
        };
        let (_, html) =
            self.t.get(&routes::with_page(&path, page), Some(LISTING_TTL), false, false).await.map_err(|e| Self::member_not_found(e, &nick))?;
        match kind {
            TextKind::Review => parse::parse_user_reviews(&html, page),
            TextKind::Statement => parse::parse_user_statements(&html, page),
        }
    }

    /// One collection list of a member (public page): every perfume of the list, with ids
    /// and pictures only. `list` is the last segment of the list URL (`I_have`, `Wish_List`,
    /// a custom list number).
    pub async fn member_collection(&self, reference: &str, list: &str) -> Result<(WardrobeCall, Vec<MemberCollectionItem>), ClientError> {
        let nick = self.member_nick(reference)?;
        let list = list.trim();
        if list.is_empty() || !list.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ClientError::InvalidInput("unknown collection list: use the `id` of a list returned by parfumo_get_user".into()));
        }
        if self.has_app_session()
            && let Some(answer) = crate::app::settle(self.app_member_collection(&nick, list).await)
        {
            return answer;
        }
        let not_found = |e| match e {
            ClientError::NotFound(_) => ClientError::NotFound(format!("collection '{list}' of member '{nick}'")),
            other => other,
        };
        let (_, html) = self.t.get(&routes::user_collection(&nick, list), Some(LISTING_TTL), false, false).await.map_err(not_found)?;
        let wr = parse::parse_wardrobe_call(&html)?;
        let form = vec![
            ("id".to_string(), wr.member_id.to_string()),
            ("nick".to_string(), wr.nick.clone()),
            ("type".to_string(), wr.list_type.clone()),
            ("type_url".to_string(), wr.type_url.clone()),
            ("gender".to_string(), String::new()),
            ("order".to_string(), String::new()),
            ("total_no_filter".to_string(), wr.total.clone()),
        ];
        let body = self.t.post_fragment(routes::GET_WARDROBE, &form, true, Some(LISTING_TTL)).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::GET_WARDROBE));
        }
        Ok((wr, parse::parse_member_wardrobe(&body)?))
    }

    /// Fills name, brand and link of one collection entry (one request per entry).
    pub async fn resolve_collection_item(&self, item: &mut MemberCollectionItem) -> Result<(), ClientError> {
        // An entry read through the application interface carries them already, and no entry id to ask with.
        if item.perfume_id.is_some() {
            return Ok(());
        }
        let form = vec![("id".to_string(), item.entry_id.to_string())];
        let body = self.t.post_fragment(routes::WARDROBE_SNEAK_PEEK, &form, true, Some(LISTING_TTL)).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::WARDROBE_SNEAK_PEEK));
        }
        let (id, name, brand, url) = parse::parse_wardrobe_peek(&body)?;
        item.perfume_id = Some(id);
        item.name = name;
        item.brand = brand;
        item.url = Some(url);
        Ok(())
    }

    /// Conversations of the mailbox, 25 at a time, most recent first. `offset` is a
    /// multiple of 25; the counters are only known on the first page.
    pub async fn mailbox(&self, offset: u32) -> Result<Mailbox, ClientError> {
        if offset == 0 {
            let (_, body) = self.t.get(&format!("{}?n=", routes::POSTBOX_CONVERSATIONS), None, true, false).await?;
            return parse::parse_mailbox(&body);
        }
        let (_, body) = self.t.get(&format!("{}?n=&f={offset}", routes::POSTBOX_CONVERSATIONS_MORE), None, true, true).await?;
        Ok(Mailbox { total: None, unread: None, items: parse::parse_conversations(&body) })
    }

    /// Numeric id and exact nickname of the other member of a conversation.
    pub async fn conversation_partner(&self, reference: &str) -> Result<(u64, String), ClientError> {
        let nick = self.member_nick(reference)?;
        let (_, html) = self.t.get(&routes::postbox_thread(&nick), None, true, false).await.map_err(|e| Self::member_not_found(e, &nick))?;
        parse::parse_thread_page(&html).map_err(|e| match e {
            // The site serves the mailbox itself for a nickname it does not know.
            ClientError::ParserOutdated(_) if !html.contains("id=\"messages\"") => ClientError::NotFound(format!("member '{nick}'")),
            other => other,
        })
    }

    /// Messages of a conversation, newest first. `before` is `(id, time)` of the oldest
    /// message already read, to get the ones before it.
    pub async fn messages(&self, member_id: u64, before: Option<(u64, i64)>, limit: u32) -> Result<Vec<PrivateMessage>, ClientError> {
        let (before_id, before_date) = before.map(|(id, time)| (id.to_string(), time.to_string())).unwrap_or_default();
        let path = format!("{}?r={member_id}&before={before_id}&before_date={before_date}&l={limit}", routes::POSTBOX_MESSAGES);
        let (_, body) = self.t.get(&path, None, true, true).await?;
        parse::parse_messages(&body)
    }

    /// Pending notifications of the account.
    pub async fn notifications(&self) -> Result<Vec<Notification>, ClientError> {
        let (_, body) = self.t.get(routes::NOTIFICATIONS, None, true, false).await?;
        let mut items = parse::parse_notifications(&body)?;
        for n in &mut items {
            n.link = n.link.take().map(|l| if l.starts_with('/') { self.t.absolute(&l) } else { l });
        }
        Ok(items)
    }

    /// Members the account follows.
    pub async fn following(&self) -> Result<Following, ClientError> {
        let (_, html) = self.t.get(routes::FOLLOW_LIST, None, true, false).await?;
        parse::parse_following(&html)
    }

    /// Whether the account follows the member with this numeric id.
    pub async fn is_following(&self, member_id: u64) -> Result<bool, ClientError> {
        let form = vec![("type".to_string(), "users".to_string()), ("item_id".to_string(), member_id.to_string())];
        let body = self.t.post_fragment(routes::SUBSCRIPTION_SUB_TYPES, &form, true, None).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::SUBSCRIPTION_SUB_TYPES));
        }
        Ok(parse::parse_sub_types(&body)?.iter().any(|t| t == "follow"))
    }

    /// Settings of one settings page, with their current values.
    pub async fn settings(&self, page: SettingsPage) -> Result<Vec<Setting>, ClientError> {
        let path = match page {
            SettingsPage::PersonalDetails => routes::SETTINGS_PERSONAL,
            SettingsPage::Notifications => routes::SETTINGS_NOTIFICATIONS,
            SettingsPage::Website => routes::SETTINGS_WEBSITE,
        };
        let (_, html) = self.t.get(path, None, true, false).await?;
        parse::parse_settings(&html)
    }

    /// The two free-text fragrance preference lines of the profile.
    pub async fn fragrance_preferences(&self) -> Result<FragrancePreferences, ClientError> {
        let (_, html) = self.t.get(routes::FRAGRANCE_PREFERENCES, None, true, false).await?;
        parse::parse_fragrance_preferences(&html)
    }

    /// What the wall form of a member posts, read on the profile page served to this
    /// session, with the exact nickname. `None`: the page offers no wall form.
    pub async fn wall_conf(&self, reference: &str) -> Result<(String, Option<CommentConf>), ClientError> {
        let nick = self.member_nick(reference)?;
        let path = routes::user(&nick);
        let (_, html) = self.t.get(&path, None, true, false).await.map_err(|e| Self::member_not_found(e, &nick))?;
        let profile = parse::parse_member_profile(&html, &self.t.absolute(&path))?;
        Ok((profile.nick, parse::parse_wall_conf(&html)))
    }

    /// Numeric `forum_id` behind the "new topic" form of a forum category, read logged in.
    /// `forum_id` is the id carried by the category page.
    pub async fn forum_compose_check(&self, forum_id: u64) -> Result<u64, ClientError> {
        let body = self.t.post_fragment(routes::FORUM_COMPOSE_PANEL, &[("forum_id".to_string(), forum_id.to_string())], true, None).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::FORUM_COMPOSE_PANEL));
        }
        parse::parse_forum_compose(&body)
    }

    /// Form of a Souk entry for a perfume: its fields with their current values, or `None`
    /// when the site does not let this account create that kind of entry.
    pub async fn souk_form(&self, p_id: u64, kind: &str, souk_id: u64) -> Result<Option<BTreeMap<String, String>>, ClientError> {
        let form =
            vec![("perfume_id".to_string(), p_id.to_string()), ("t".to_string(), kind.to_string()), ("souk_id".to_string(), souk_id.to_string())];
        let body = self.t.post_fragment(routes::SOUK_ITEM_PANEL, &form, true, None).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::SOUK_ITEM_PANEL));
        }
        Ok(parse::parse_souk_form(&body))
    }
}
