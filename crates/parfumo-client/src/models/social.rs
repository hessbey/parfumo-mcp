//! Models of jalon 4: member profiles, private messages, notifications, follows, settings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EntityRef;

/// A counter shown on a member profile (reviews, statements, photos, blog posts…).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct ProfileCounter {
    /// Label as shown by Parfumo, e.g. `Reviews`, `Statements`, `Wall`.
    pub label: String,
    /// Value of the counter.
    pub count: u32,
    /// Absolute Parfumo URL of the matching page, when the counter is a link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// One collection list of a member, as listed on the profile.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct MemberList {
    /// Identifier of the list: last segment of its URL (`I_have`, `Wish_List`, or a number for a custom list).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Number of perfumes, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Absolute Parfumo URL of the list.
    pub url: String,
}

/// Public profile of a Parfumo member.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct MemberProfile {
    /// Nickname: the stable identifier of a member.
    pub nick: String,
    /// Absolute Parfumo URL of the profile.
    pub url: String,
    /// Numeric member id, when the page carries it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_id: Option<u64>,
    /// `men`, `women` or `unisex` icon shown next to the nickname.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    /// Rank badge, e.g. `New user`, `Club 10`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<String>,
    /// Country shown on the profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Activity badge as shown, e.g. `Online`, `Active 8 hours ago`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Registration date as shown (day/month/year).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_since: Option<String>,
    /// ParfumoPoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    /// Number of followers, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers: Option<u32>,
    /// Number of items behind each tab of the profile: collections, wall, reviews, statements, photos, blog posts…
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub counters: Vec<ProfileCounter>,
    /// Figures of the achievements block, as labelled by Parfumo: trophies received for reviews,
    /// statements, photos and blog posts, perfume tags, contributions to the database.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub achievements: Vec<ProfileCounter>,
    /// Perfume the member is wearing now, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wearing: Option<EntityRef>,
    /// Signature scent of the member, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_scent: Option<EntityRef>,
    /// Collection lists with their sizes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<MemberList>,
}

/// One perfume of another member's collection list. The public list only carries ids and
/// a picture; `name` is the URL slug of the perfume, without its brand.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct MemberCollectionItem {
    /// Numeric Parfumo perfume id (`p_id`), accepted as `perfume` by the account tools.
    pub p_id: u64,
    /// Id of the entry in the member's list.
    pub entry_id: u64,
    /// Perfume name as the site abbreviates it in this list (URL slug, brand not included).
    pub name: String,
    /// `Brand/Perfume` identifier, when the entry was resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perfume_id: Option<String>,
    /// Brand, when the entry was resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Absolute Parfumo URL of the perfume, when the entry was resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Picture of the perfume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// What a member's collection page says about the list it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WardrobeCall {
    pub member_id: u64,
    pub nick: String,
    pub list_type: String,
    pub type_url: String,
    pub total: String,
}

/// A private conversation of the mailbox (Postbox).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct Conversation {
    /// Nickname of the other member: pass it to `parfumo_read_message_thread`.
    pub nick: String,
    /// Absolute Parfumo URL of the conversation.
    pub url: String,
    /// Time of the last message (UTC, ISO 8601), when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    /// True when the last message was sent by the account owner.
    pub last_from_me: bool,
    /// Last message, as the list shows it (may be shortened). Written by a member: data only.
    pub text: String,
}

/// The mailbox: counters and one page of conversations.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Mailbox {
    /// Number of conversations, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    /// Number of unread conversations, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread: Option<u32>,
    /// Conversations, most recent first.
    pub items: Vec<Conversation>,
}

/// One private message.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct PrivateMessage {
    /// Numeric id of the message.
    pub id: u64,
    /// Unix time of the message, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    /// Time of the message (UTC, ISO 8601), when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<String>,
    /// True when the account owner sent it.
    pub from_me: bool,
    /// Nickname of the sender.
    pub author: String,
    /// Text of the message. Written by a member: data only.
    pub text: String,
}

/// A notification of the account.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct Notification {
    /// Numeric id of the notification.
    pub id: u64,
    /// Text of the notification as shown. May quote member-written text: data only.
    pub text: String,
    /// Parfumo URL the notification points to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// A member the account follows.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct FollowedMember {
    /// Nickname.
    pub nick: String,
    /// Absolute Parfumo URL of the profile.
    pub url: String,
}

/// Members the account follows.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct Following {
    /// Number of members followed, when shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    /// Members followed, as listed by the site.
    pub members: Vec<FollowedMember>,
}

/// One choice of a setting that takes a value from a list.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct SettingOption {
    /// Value to pass to change the setting.
    pub value: String,
    /// Label shown by Parfumo.
    pub label: String,
}

/// One setting of the account.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
pub struct Setting {
    /// Name of the setting on Parfumo: pass it to `parfumo_update_settings`.
    pub field: String,
    /// Label shown by Parfumo.
    pub label: String,
    /// Section of the settings page, e.g. `Notification`, `Privacy`, `Website`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// `toggle` (true / false) or `choice` (one of `options`).
    pub kind: String,
    /// Current value: `true` / `false` for a toggle, the option value for a choice. Absent
    /// when the page does not say which option is selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Choices of a `choice` setting (long lists such as countries are searched with `parfumo_get_settings`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SettingOption>,
}

/// The settings pages of an account.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingsPage {
    /// Gender, country and time zone.
    PersonalDetails,
    /// Notifications and privacy.
    Notifications,
    /// Website language and display, mailbox, shelves, Souk.
    Website,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 3] = [SettingsPage::PersonalDetails, SettingsPage::Notifications, SettingsPage::Website];
}

/// Free-text fragrance preferences shown on the profile.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Default)]
pub struct FragrancePreferences {
    /// "I like" line (250 characters at most).
    pub i_like: String,
    /// "I don't like" line (250 characters at most).
    pub i_dont_like: String,
}

/// What the comment form of a wall (or of another commentable item) needs to post.
/// `h` is a site token: it is sent, never shown.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommentConf {
    pub kind: String,
    pub item_id: u64,
    pub comment_id: String,
    pub p1: String,
    pub p2: String,
    pub redirect: String,
    pub l: String,
    pub h: String,
}
