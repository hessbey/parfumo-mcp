//! Parsers of jalon 4: member profiles, another member's collection, mailbox,
//! notifications, follows, settings and the forms of the public writes.
//! Selectors live in `site.rs`, like those of the parent module.

use std::collections::BTreeMap;

use scraper::{ElementRef, Html};

use super::*;

/// `YYYY-MM-DDTHH:MM:SSZ` of a Unix timestamp.
pub fn iso_datetime(ts: i64) -> String {
    let secs = ts.rem_euclid(86_400);
    format!("{}T{:02}:{:02}:{:02}Z", iso_date(ts), secs / 3600, secs % 3600 / 60, secs % 60)
}

fn perfume_ref(link: ElementRef) -> Option<EntityRef> {
    let href = link.value().attr("href")?;
    Some(EntityRef { id: perfume_id_from_url(href)?, numeric_id: None, name: text_of(link), url: href.to_string() })
}

fn last_segment(url: &str) -> Option<String> {
    url_path(url).rsplit('/').find(|p| !p.is_empty()).map(str::to_string)
}

/// Profile page of a member (`/Users/{nick}`), logged in or not.
pub fn parse_member_profile(html: &str, url: &str) -> PResult<MemberProfile> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let head = first(root, sel::PROFILE_HEAD).ok_or_else(|| outdated("member profile: heading not found"))?;
    let nick = text_of(head);
    if nick.is_empty() {
        return Err(outdated("member profile: empty nickname"));
    }
    let mut p = MemberProfile { nick, url: url.to_string(), ..Default::default() };
    p.gender = first(root, sel::PROFILE_GENDER_ICON).and_then(|i| gender_from_classes(i.value().classes()));
    for badge in root.select(&s(sel::PROFILE_BADGE)) {
        let text = text_of(badge);
        if text.is_empty() {
            continue;
        }
        if first(badge, sel::PROFILE_BADGE_FLAG).is_some() {
            p.country = Some(text);
        } else if first(badge, sel::PROFILE_BADGE_ACTIVITY).is_some() {
            p.activity = Some(text);
        } else if p.rank.is_none() {
            p.rank = Some(text);
        }
    }
    p.points = first(root, sel::PROFILE_POINTS).and_then(|a| parse_int(&text_of(a)));
    p.member_since = first(root, sel::PROFILE_SCORE).and_then(|e| rx(re::MEMBER_SINCE).captures(&text_of(e)).map(|c| c[1].to_string()));
    p.followers = first(root, sel::PROFILE_PIC_BOX).and_then(|e| rx(re::FOLLOWERS).captures(&text_of(e)).and_then(|c| parse_int(&c[1])));
    p.member_id = first(root, sel::PROFILE_WALL_HOLDER)
        .and_then(|e| e.value().attr("id"))
        .and_then(|id| rx(re::WALL_HOLDER_ID).captures(id).and_then(|c| c[1].parse().ok()))
        .or_else(|| attr(root, sel::PROFILE_SUBSCRIBE, "data-item-id").and_then(|id| id.parse().ok()))
        .or_else(|| rx(re::WALL_BUTTON_ID).captures(html).and_then(|c| c[1].parse().ok()));

    let counter = |label: String, count: Option<u32>, url: Option<&str>| {
        count.filter(|_| !label.is_empty()).map(|count| ProfileCounter { label, count, url: url.map(str::to_string) })
    };
    for tab in root.select(&s(sel::PROFILE_TAB)) {
        let count = first(tab, sel::PROFILE_TAB_COUNT).and_then(|c| parse_int(&text_of(c)));
        p.counters.extend(counter(own_text(tab), count, tab.value().attr("href")));
    }
    for item in root.select(&s(sel::PROFILE_ACHIEVEMENT)) {
        let Some(number) = first(item, sel::PROFILE_ACHIEVEMENT_COUNT) else { continue };
        p.achievements.extend(counter(own_text(item), parse_int(&text_of(number)), item.value().attr("href")));
    }

    p.wearing = first(root, sel::PROFILE_WEARING).and_then(perfume_ref);
    p.signature_scent = first(root, sel::PROFILE_SIGNATURE).and_then(perfume_ref);
    for a in root.select(&s(sel::PROFILE_LIST)) {
        let Some(href) = a.value().attr("href") else { continue };
        let Some(id) = last_segment(href) else { continue };
        let name = a.children().filter_map(ElementRef::wrap).find(|c| c.value().name() == "div").map(own_text).unwrap_or_default();
        let name = if name.is_empty() { id.replace('_', " ") } else { name.replace("\\'", "'") };
        p.collections.push(MemberList {
            id,
            name,
            count: first(a, sel::PROFILE_LIST_COUNT).and_then(|c| parse_int(&text_of(c))),
            url: href.to_string(),
        });
    }
    Ok(p)
}

/// What a member's collection page asks `get_wardrobe.php` for.
pub fn parse_wardrobe_call(html: &str) -> PResult<WardrobeCall> {
    let caps = rx(re::WARDROBE_CALL).captures(html).ok_or_else(|| outdated("member collection page: list call not found"))?;
    Ok(WardrobeCall {
        member_id: caps[1].parse().map_err(|_| outdated("member collection page: member id"))?,
        nick: caps[2].to_string(),
        list_type: caps[3].to_string(),
        type_url: caps[4].to_string(),
        total: caps[5].to_string(),
    })
}

/// Perfumes of a member's collection list (`get_wardrobe.php` fragment).
pub fn parse_member_wardrobe(fragment: &str) -> PResult<Vec<MemberCollectionItem>> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    if first(root, sel::WARDROBE_WRAPPER).is_none() {
        return Err(outdated("member collection list: wrapper not found"));
    }
    Ok(root
        .select(&s(sel::WARDROBE_ITEM))
        .filter_map(|item| {
            let v = item.value();
            Some(MemberCollectionItem {
                p_id: v.attr("data-id")?.parse().ok()?,
                entry_id: v.attr("data-wr-id")?.parse().ok()?,
                name: squash(&v.attr("alt").unwrap_or_default().replace(['_', '-'], " ")),
                perfume_id: None,
                brand: None,
                url: None,
                image_url: attr(item, "img", "src").map(str::to_string),
            })
        })
        .collect())
}

/// Name, brand and link of one collection entry (`wardrobe_sneak_peek.php` fragment):
/// `(perfume id, name, brand, url)`.
pub fn parse_wardrobe_peek(fragment: &str) -> PResult<(String, String, Option<String>, String)> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let link = first(root, sel::WARDROBE_PEEK_LINK).ok_or_else(|| outdated("collection entry preview: perfume link not found"))?;
    let href = link.value().attr("href").unwrap_or_default();
    let id = perfume_id_from_url(href).ok_or_else(|| outdated("collection entry preview: perfume link"))?;
    Ok((id, text_of(link), first(root, sel::WARDROBE_PEEK_BRAND).map(text_of).and_then(none_if_empty), href.to_string()))
}

/// Conversation rows of a mailbox fragment (first page or a following one).
pub fn parse_conversations(fragment: &str) -> Vec<Conversation> {
    let doc = Html::parse_fragment(fragment);
    doc.root_element()
        .select(&s(sel::POSTBOX_CONVERSATION))
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let nick = last_segment(href)?;
            let message = first(a, sel::POSTBOX_CONVERSATION_TEXT)?;
            Some(Conversation {
                nick,
                url: href.to_string(),
                last_message_at: attr(a, sel::POSTBOX_CONVERSATION_DATE, "data-time").and_then(|t| t.parse::<i64>().ok()).map(iso_datetime),
                last_from_me: first(a, sel::POSTBOX_CONVERSATION_SENT).is_some(),
                text: own_text(message),
            })
        })
        .collect()
}

/// First page of the mailbox (`_get_postbox_conv.php`): counters and conversations.
pub fn parse_mailbox(fragment: &str) -> PResult<Mailbox> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    if first(root, sel::POSTBOX_VIEW).is_none() {
        return Err(outdated("mailbox: conversation list not found"));
    }
    let counts: Vec<Option<u32>> = root.select(&s(sel::POSTBOX_TAB_COUNT)).map(|c| parse_int(&text_of(c))).collect();
    Ok(Mailbox { total: counts.first().copied().flatten(), unread: counts.get(1).copied().flatten(), items: parse_conversations(fragment) })
}

/// Conversation page (`/Postbox/R/{nick}`): numeric id and nickname of the other member.
pub fn parse_thread_page(html: &str) -> PResult<(u64, String)> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let id = attr(root, sel::POSTBOX_THREAD, "data-user").and_then(|v| v.parse::<u64>().ok()).filter(|id| *id > 0);
    let nick = first(root, sel::POSTBOX_THREAD_NICK).map(text_of).filter(|n| !n.is_empty());
    match (id, nick) {
        (Some(id), Some(nick)) => Ok((id, nick)),
        _ => Err(outdated("conversation page: member id or nickname not found")),
    }
}

/// Message rows of a conversation (`_get_postbox.php`), in the order served (newest first).
/// An empty answer means no (more) messages.
pub fn parse_messages(fragment: &str) -> PResult<Vec<PrivateMessage>> {
    if fragment.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Rows are bare `<tr>`: they only parse inside a table.
    let doc = Html::parse_fragment(&format!("<table>{fragment}</table>"));
    let rows: Vec<PrivateMessage> = doc
        .root_element()
        .select(&s(sel::POSTBOX_MESSAGE))
        .filter_map(|row| {
            let v = row.value();
            let text = first(row, sel::POSTBOX_MESSAGE_TEXT)?;
            let time = v.attr("data-time").and_then(|t| t.parse::<i64>().ok());
            Some(PrivateMessage {
                id: v.attr("data-id")?.parse().ok()?,
                time,
                sent_at: time.map(iso_datetime),
                from_me: v.classes().any(|c| c == sel::POSTBOX_MESSAGE_SENT_CLASS),
                author: text.value().attr("data-quote-author").unwrap_or_default().to_string(),
                text: rich_text(text, None),
            })
        })
        .collect();
    if rows.is_empty() { Err(outdated("conversation: no message row in a non-empty answer")) } else { Ok(rows) }
}

/// Pending notifications (`notification/get_history` fragment).
pub fn parse_notifications(fragment: &str) -> PResult<Vec<Notification>> {
    let doc = Html::parse_fragment(fragment);
    let root = doc.root_element();
    let items: Vec<Notification> = root
        .select(&s(sel::NOTIFICATION_ITEM))
        .filter_map(|item| {
            Some(Notification {
                id: item.value().attr("data-id")?.parse().ok()?,
                text: squash(&rich_text(item, Some(sel::NOTIFICATION_SKIP_CLASS))),
                link: item.value().attr("data-link").map(str::to_string).and_then(none_if_empty),
            })
        })
        .collect();
    if items.is_empty() && first(root, sel::NOTIFICATION_EMPTY).is_none() {
        return Err(outdated("notifications: neither an item nor the empty marker"));
    }
    Ok(items)
}

/// Members the account follows (`/account/follow/list`, or the strip of `/account/follow`).
pub fn parse_following(html: &str) -> PResult<Following> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    // Nobody followed: the page has no tabs, only the "No Users found" picture.
    if first(root, sel::FOLLOW_TABS).is_none() && first(root, sel::FOLLOW_EMPTY).is_none() {
        return Err(outdated("follow page: neither the tabs nor the empty marker"));
    }
    let mut members: Vec<FollowedMember> = Vec::new();
    for a in root.select(&s(sel::FOLLOW_MEMBER)) {
        let Some(href) = a.value().attr("href") else { continue };
        // Only links to a profile itself, not to a member's review or blog.
        let Some(rest) = url_path(href).strip_prefix(routes::USERS_PREFIX) else { continue };
        if rest.is_empty() || rest.contains('/') || members.iter().any(|m| m.nick == rest) {
            continue;
        }
        members.push(FollowedMember { nick: rest.to_string(), url: href.to_string() });
    }
    Ok(Following { total: first(root, sel::FOLLOW_TAB_COUNT).and_then(|c| parse_int(&text_of(c))), members })
}

/// Sub types the member subscribed to for an item (`subscription_sub_types`): a JSON array.
pub fn parse_sub_types(body: &str) -> PResult<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).map_err(|_| outdated("subscriptions: answer is not JSON"))?;
    match v {
        serde_json::Value::Array(a) => Ok(a.into_iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
        serde_json::Value::Object(o) => Ok(o.into_iter().filter_map(|(_, x)| x.as_str().map(str::to_string)).collect()),
        _ => Err(outdated("subscriptions: unexpected JSON")),
    }
}

/// Every setting of a settings page, in page order, with its section heading.
pub fn parse_settings(html: &str) -> PResult<Vec<Setting>> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let all = format!("{}, {}, {}", sel::SETTING_SECTION, sel::SETTING_TOGGLE, sel::SETTING_SELECT);
    let mut section: Option<String> = None;
    let mut out: Vec<Setting> = Vec::new();
    for el in root.select(&s(&all)) {
        let v = el.value();
        if v.name() == "h2" {
            section = none_if_empty(text_of(el));
            continue;
        }
        let Some(field) = v.attr("data-field").map(str::trim).filter(|f| !f.is_empty()) else { continue };
        if out.iter().any(|x| x.field == field) {
            continue;
        }
        let row = el.ancestors().filter_map(ElementRef::wrap).find(|a| a.value().name() == sel::SETTING_ROW);
        let label = row.and_then(|r| first(r, sel::SETTING_LABEL)).map(own_text).and_then(none_if_empty).unwrap_or_else(|| field.replace('_', " "));
        let setting = if v.name() == "select" {
            let options: Vec<(SettingOption, bool)> = el
                .select(&s(sel::SETTING_OPTION))
                .filter_map(|o| {
                    Some((
                        SettingOption { value: o.value().attr("data-value")?.to_string(), label: text_of(o) },
                        o.value().attr("selected").is_some(),
                    ))
                })
                .collect();
            let value = options.iter().find(|(_, selected)| *selected).map(|(o, _)| o.value.clone());
            Setting {
                field: field.to_string(),
                label,
                section: section.clone(),
                kind: "choice".into(),
                value,
                options: options.into_iter().map(|(o, _)| o).collect(),
            }
        } else {
            Setting {
                field: field.to_string(),
                label,
                section: section.clone(),
                kind: "toggle".into(),
                value: Some(v.attr("checked").is_some().to_string()),
                options: Vec::new(),
            }
        };
        out.push(setting);
    }
    if out.is_empty() { Err(outdated("settings page: no setting found")) } else { Ok(out) }
}

/// The two lines of the fragrance preferences form.
pub fn parse_fragrance_preferences(html: &str) -> PResult<FragrancePreferences> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let value = |css: &str| attr(root, css, "value").map(|v| v.trim().to_string());
    match (value(sel::PREFERENCES_LIKE), value(sel::PREFERENCES_DISLIKE)) {
        (Some(i_like), Some(i_dont_like)) => Ok(FragrancePreferences { i_like, i_dont_like }),
        _ => Err(outdated("fragrance preferences: form fields not found")),
    }
}

/// Signature scent shown by the fragment `account/signature_scent` answers.
pub fn parse_signature_fragment(fragment: &str) -> Option<EntityRef> {
    let doc = Html::parse_fragment(fragment);
    first(doc.root_element(), sel::SIGNATURE_FRAGMENT_LINK).and_then(perfume_ref)
}

/// What the wall form of a profile page posts (page served to a session). `None` when the
/// page carries no such form: the site does not offer this account to write there.
pub fn parse_wall_conf(html: &str) -> Option<CommentConf> {
    let c = rx(re::WALL_CONF).captures(html)?;
    Some(CommentConf {
        kind: routes::COMMENT_TYPE_WALL.to_string(),
        item_id: c[1].parse().ok()?,
        comment_id: c[2].to_string(),
        redirect: c[3].to_string(),
        p1: c[4].to_string(),
        p2: c[5].to_string(),
        l: c[6].to_string(),
        h: c[7].to_string(),
    })
}

/// `forum_id` of the "new topic" form of a forum category.
pub fn parse_forum_compose(fragment: &str) -> PResult<u64> {
    let doc = Html::parse_fragment(fragment);
    attr(doc.root_element(), sel::FORUM_COMPOSE_FORM, "value")
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| outdated("forum: new topic form not found"))
}

/// Fields of the Souk entry form with their current values. `None` when the panel shows
/// no form: the site does not let this account create that kind of entry.
pub fn parse_souk_form(fragment: &str) -> Option<BTreeMap<String, String>> {
    let doc = Html::parse_fragment(fragment);
    let form = first(doc.root_element(), sel::SOUK_FORM)?;
    Some(
        form.select(&s(sel::SOUK_FORM_FIELD))
            .filter_map(|f| {
                let name = f.value().attr("name")?.to_string();
                let value = if f.value().name() == "textarea" {
                    f.text().collect::<String>().trim().to_string()
                } else {
                    f.value().attr("value").unwrap_or_default().to_string()
                };
                Some((name, value))
            })
            .collect(),
    )
}
