//! Profile and account tools of jalon 4 (section 7.6): own profile, profile fields,
//! settings, and the state of the link with its daily quotas.
//!
//! Measured logged in at jalon 0 bis (DISCOVERY §3.7): the settings pages, one setting
//! toggled and restored, fragrance preferences filled and emptied, signature scent set and
//! removed. Gender, country, the two preference lines and the signature scent show on the
//! public profile (type P); the other settings are private (type E).

use std::time::Instant;

use parfumo_client::{EntityRef, FragrancePreferences, MemberProfile, Setting, SettingOption, SettingsPage, WriteCall, site::routes, social_calls};
use rmcp::{
    ErrorData,
    handler::server::{common::schema_for_output, wrapper::Parameters},
    model::CallToolResult,
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    account_tools::{NoParams, chars_at_most, planned_text},
    caller::Caller,
    errors::{ErrorCode, ToolError},
    paging,
    public_gate::{self, GATE_NOTE, OWN_WORDS_NOTE, Quota},
    sessions::SessionError,
    tools::{Clients, ParfumoTools},
};

/// Choices listed in full for a setting; longer lists (countries, time zones) are searched.
const MAX_OPTIONS_SHOWN: usize = 30;
/// The account language is not changed by this server: its parsers read the English site.
const LANGUAGE_FIELD: &str = "reg_lang";

// ---- parameters ----------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateProfileParams {
    /// Gender shown on the profile: "male", "female" or "non-binary" (or the site value m / f / d).
    #[serde(default)]
    pub gender: Option<String>,
    /// Country shown on the profile, by its English name as Parfumo lists it (e.g. "Switzerland").
    #[serde(default)]
    pub country: Option<String>,
    /// Time zone of the account, e.g. "Europe / Zurich".
    #[serde(default)]
    pub time_zone: Option<String>,
    /// "I like" line of the fragrance preferences (250 characters at most), in the user's own words. An empty string clears it.
    #[serde(default)]
    pub i_like: Option<String>,
    /// "I don't like" line of the fragrance preferences (250 characters at most), in the user's own words. An empty string clears it.
    #[serde(default)]
    pub i_dont_like: Option<String>,
    /// Signature scent: a perfume `id`, Parfumo URL or numeric `p_id`; "none" removes it.
    #[serde(default)]
    pub signature_scent: Option<String>,
    /// When true, nothing is sent: the answer shows the requests that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSettingsParams {
    /// Settings page to read: `notifications` (notifications and privacy), `website` or `personal_details`. Default: notifications and website.
    #[serde(default)]
    pub page: Option<SettingsPage>,
    /// Name of a setting whose choices are wanted in full (e.g. "country_id", "time_zone_id").
    #[serde(default)]
    pub options_of: Option<String>,
    /// With `options_of`: only the choices whose label contains this text.
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSettingsParams {
    /// Name of the setting, as returned by parfumo_get_settings (e.g. "notifications_messages", "hide_followers").
    pub field: String,
    /// New value: "true" / "false" for a toggle, the value or the label of an option for a choice.
    pub value: String,
    /// When true, nothing is sent: the answer shows the request that would be made.
    #[serde(default)]
    pub dry_run: bool,
}

// ---- outputs -------------------------------------------------------------------------------

/// One setting with its current value.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct SettingOut {
    /// Settings page it belongs to.
    pub page: SettingsPage,
    /// Name of the setting: pass it to parfumo_update_settings.
    pub field: String,
    /// Label shown by Parfumo.
    pub label: String,
    /// Section of the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// `toggle` (true / false) or `choice`.
    pub kind: String,
    /// Current value; absent when the page does not mark the selected choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Label of the current choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_label: Option<String>,
    /// Choices of a `choice` setting; omitted when there are more than 30 (use `options_of`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SettingOption>,
    /// Number of choices of a `choice` setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options_total: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MyProfileOutput {
    /// The account's profile as Parfumo shows it: rank, points, counters, lists.
    pub profile: MemberProfile,
    /// The two fragrance preference lines of the profile.
    pub fragrance_preferences: FragrancePreferences,
    /// Gender, country and time zone of the account.
    pub personal_details: Vec<SettingOut>,
}

#[derive(Debug, Serialize, JsonSchema, Default)]
pub struct ProfileWriteOutput {
    /// True when nothing was sent to Parfumo.
    pub dry_run: bool,
    /// True when at least one write request was sent.
    pub executed: bool,
    /// Outcome: `updated`, `unchanged`, `unverified` or `dry_run`.
    pub status: String,
    /// Whether this server is allowed to send publicly visible writes (operator setting).
    pub public_writes_enabled: bool,
    /// The write requests sent, or that would be sent.
    pub planned_requests: Vec<WriteCall>,
    /// Gender, country and time zone as Parfumo holds them after the call (before it in a dry run).
    pub personal_details: Vec<SettingOut>,
    /// Fragrance preferences after the call; present when the call was about them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragrance_preferences: Option<FragrancePreferences>,
    /// Signature scent after the call; present when the call was about it and one is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_scent: Option<EntityRef>,
    /// Side effects and things to know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SettingsOutput {
    /// Settings with their current values.
    pub settings: Vec<SettingOut>,
    /// With `options_of`: the choices of that setting (filtered by `search`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SettingOption>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SettingWriteOutput {
    /// True when nothing was sent to Parfumo.
    pub dry_run: bool,
    /// True when the write request was sent.
    pub executed: bool,
    /// Outcome: `updated`, `unchanged`, `unverified` or `dry_run`.
    pub status: String,
    /// The write request sent, or that would be sent.
    pub planned_requests: Vec<WriteCall>,
    /// The setting as Parfumo holds it after the call (before it in a dry run).
    pub setting: SettingOut,
    /// Things to know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionStatusOutput {
    /// Whether a Parfumo account is linked for this user.
    pub linked: bool,
    /// `credentials` (the server logs in again by itself) or `cookie` (the user pastes a new cookie when it expires).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// State of the Parfumo session right now: `valid`, `not_linked`, `reauth_required`, `captcha_required` or `error`.
    pub session: String,
    /// Nickname of the linked Parfumo account, when the session is valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick: Option<String>,
    /// When the account was linked (UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_at: Option<String>,
    /// Last login of the server on Parfumo (UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_login_at: Option<String>,
    /// Page where the user links or relinks the account: the `/link` page of an HTTP server,
    /// or the login page a local stdio server opens on the user's own computer when the
    /// account has to be linked. Absent when there is nothing to link, or when the account
    /// of a local server comes from the environment of the process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_url: Option<String>,
    /// Whether this server sends publicly visible writes at all; when false they are dry runs.
    pub public_writes_enabled: bool,
    /// Today's budget of each tool with publicly visible writes, for this user.
    pub quotas: Vec<Quota>,
    /// Seconds until the daily counts reset (00:00 UTC).
    pub quotas_reset_in_secs: u64,
    /// Members this server may write to (message, wall, follow); empty when unrestricted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub write_recipients: Vec<String>,
    /// Minimum interval between two requests of this user to Parfumo, in milliseconds.
    pub min_request_interval_ms: u64,
}

// ---- helpers -------------------------------------------------------------------------------

fn setting_out(page: SettingsPage, s: Setting) -> SettingOut {
    let value_label = s.value.as_ref().and_then(|v| s.options.iter().find(|o| &o.value == v)).map(|o| o.label.clone());
    let total = s.options.len();
    SettingOut {
        page,
        field: s.field,
        label: s.label,
        section: s.section,
        kind: s.kind,
        value: s.value,
        value_label,
        options: if total <= MAX_OPTIONS_SHOWN { s.options } else { Vec::new() },
        options_total: (total > 0).then_some(total as u32),
    }
}

fn setting_line(s: &SettingOut) -> String {
    format!(
        "{} = {}{} — {}",
        s.field,
        s.value.as_deref().unwrap_or("?"),
        s.value_label.as_ref().map(|l| format!(" ({l})")).unwrap_or_default(),
        s.label
    )
}

/// Finds the option a caller means: by value, by label, then by a label that contains the text.
fn pick_option<'a>(setting: &'a Setting, wanted: &str) -> Result<&'a SettingOption, ToolError> {
    let w = wanted.trim().to_lowercase();
    let squeeze = |s: &str| s.to_lowercase().replace([' ', '-', '_', '/'], "");
    if let Some(o) = setting.options.iter().find(|o| o.value.to_lowercase() == w || o.label.to_lowercase() == w) {
        return Ok(o);
    }
    let hits: Vec<&SettingOption> = setting.options.iter().filter(|o| !w.is_empty() && squeeze(&o.label).contains(&squeeze(&w))).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(ToolError::invalid(format!(
            "'{}' is not a choice of `{}`.{}",
            wanted.trim(),
            setting.field,
            if setting.options.len() <= MAX_OPTIONS_SHOWN {
                format!(" Choices: {}.", setting.options.iter().map(|o| format!("{} ({})", o.label, o.value)).collect::<Vec<_>>().join(", "))
            } else {
                format!(" Call parfumo_get_settings with options_of: \"{}\" and `search` to find it.", setting.field)
            }
        ))),
        many => Err(ToolError::invalid(format!(
            "'{}' matches several choices of `{}`: {}. Give the exact label.",
            wanted.trim(),
            setting.field,
            many.iter().take(10).map(|o| o.label.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn toggle_value(wanted: &str) -> Result<&'static str, ToolError> {
    match wanted.trim().to_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok("true"),
        "false" | "off" | "no" | "0" => Ok("false"),
        _ => Err(ToolError::invalid("A toggle takes \"true\" or \"false\".")),
    }
}

fn gender_value(wanted: &str) -> &str {
    match wanted.trim().to_lowercase().as_str() {
        "male" | "man" | "men" | "m" => "m",
        "female" | "woman" | "women" | "f" => "f",
        "non-binary" | "nonbinary" | "non binary" | "diverse" | "d" => "d",
        _ => wanted,
    }
}

fn iso(ts: i64) -> String {
    parfumo_client::parse::iso_datetime(ts)
}

impl ParfumoTools {
    async fn personal_details(&self, c: &Clients) -> Result<Vec<Setting>, ToolError> {
        c.account.settings(SettingsPage::PersonalDetails).await.map_err(|e| self.ce(e))
    }
}

// ---- tools ---------------------------------------------------------------------------------

#[tool_router(router = profile_router, vis = "pub(crate)")]
impl ParfumoTools {
    #[tool(
        name = "parfumo_get_my_profile",
        description = "Get the user's own Parfumo profile: nickname, rank, ParfumoPoints, registration date, counters (collections, wall, reviews, statements...), perfume worn, signature scent, collection lists, the two fragrance preference lines, and gender, country and time zone. Requires a linked Parfumo account.",
        annotations(title = "Get my profile", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<MyProfileOutput>()
    )]
    async fn get_my_profile(&self, Parameters(_p): Parameters<NoParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        self.run_account(&caller, "parfumo_get_my_profile", "own profile", |c| async move {
            let nick = c.account.own_nick().await.map_err(|e| self.ce(e))?;
            let profile = c.account.member_profile(&nick, false).await.map_err(|e| self.ce(e))?;
            let fragrance_preferences = c.account.fragrance_preferences().await.map_err(|e| self.ce(e))?;
            let personal_details: Vec<SettingOut> =
                self.personal_details(&c).await?.into_iter().map(|s| setting_out(SettingsPage::PersonalDetails, s)).collect();
            let mut lines = vec![format!(
                "{}{}{}{}",
                profile.url,
                profile.rank.as_ref().map(|r| format!(" · {r}")).unwrap_or_default(),
                profile.points.map(|n| format!(" · {n} points")).unwrap_or_default(),
                profile.member_since.as_ref().map(|d| format!(" · member since {d}")).unwrap_or_default()
            )];
            if !profile.counters.is_empty() {
                lines.push(profile.counters.iter().map(|c| format!("{} {}", c.label, c.count)).collect::<Vec<_>>().join(", "));
            }
            lines.extend(profile.signature_scent.as_ref().map(|s| format!("Signature scent: {} (id: {})", s.name, s.id)));
            lines.extend(profile.wearing.as_ref().map(|w| format!("Wearing: {} (id: {})", w.name, w.id)));
            lines.push(format!(
                "I like: {} · I don't like: {}",
                if fragrance_preferences.i_like.is_empty() { "-" } else { &fragrance_preferences.i_like },
                if fragrance_preferences.i_dont_like.is_empty() { "-" } else { &fragrance_preferences.i_dont_like }
            ));
            lines.extend(personal_details.iter().map(setting_line));
            let text = paging::digest(format!("Profile of {}", profile.nick), lines);
            Ok((text, MyProfileOutput { profile, fragrance_preferences, personal_details }))
        })
        .await
    }

    #[tool(
        name = "parfumo_update_profile",
        description = "Change what the user's own Parfumo profile shows: gender, country, time zone, the two fragrance preference lines (`i_like`, `i_dont_like`, 250 characters each) and the signature scent. Only the fields given are changed. PUBLIC: these show on the member's profile. Parfumo forbids AI-generated text: the preference lines must be the user's own words, passed on unchanged. Parfumo has no free presentation text, no favourite perfumes list, and the avatar is not handled. Returns the fields as Parfumo holds them after the call. On this server publicly visible writes stay in dry-run mode unless the operator enabled them, with a daily cap per user. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Update my profile", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<ProfileWriteOutput>()
    )]
    async fn update_profile(&self, Parameters(p): Parameters<UpdateProfileParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let (dry_run, public_writes) = self.public_gate(p.dry_run);
        let given: Vec<&str> = [
            ("gender", p.gender.is_some()),
            ("country", p.country.is_some()),
            ("time_zone", p.time_zone.is_some()),
            ("i_like", p.i_like.is_some()),
            ("i_dont_like", p.i_dont_like.is_some()),
            ("signature_scent", p.signature_scent.is_some()),
        ]
        .iter()
        .filter(|(_, set)| *set)
        .map(|(name, _)| *name)
        .collect();
        let target = format!("{} dry_run={dry_run}", given.join(","));
        let given = &given;
        self.run_account(&caller, "parfumo_update_profile", &target, |c| async move {
            if given.is_empty() {
                return Err(ToolError::invalid("Give at least one of: gender, country, time_zone, i_like, i_dont_like, signature_scent."));
            }
            let personal = self.personal_details(&c).await?;
            let mut planned: Vec<WriteCall> = Vec::new();
            let mut wanted_settings: Vec<(&str, String)> = Vec::new();
            for (field, wanted) in
                [("gender", p.gender.as_deref().map(gender_value)), ("country_id", p.country.as_deref()), ("time_zone_id", p.time_zone.as_deref())]
            {
                let Some(wanted) = wanted else { continue };
                let setting = personal
                    .iter()
                    .find(|s| s.field == field)
                    .ok_or_else(|| self.ce(parfumo_client::ClientError::ParserOutdated(format!("personal details: `{field}` not found"))))?;
                let option = pick_option(setting, wanted)?;
                if setting.value.as_deref() != Some(option.value.as_str()) {
                    planned.push(social_calls::setting_set(field, &option.value));
                }
                wanted_settings.push((field, option.value.clone()));
            }
            let about_prefs = p.i_like.is_some() || p.i_dont_like.is_some();
            let mut prefs_before = None;
            let mut wanted_prefs = None;
            if about_prefs {
                let current = c.account.fragrance_preferences().await.map_err(|e| self.ce(e))?;
                let wanted = FragrancePreferences {
                    i_like: p.i_like.as_deref().map(str::trim).unwrap_or(&current.i_like).to_string(),
                    i_dont_like: p.i_dont_like.as_deref().map(str::trim).unwrap_or(&current.i_dont_like).to_string(),
                };
                chars_at_most("i_like", &wanted.i_like, routes::PREFERENCE_MAX)?;
                chars_at_most("i_dont_like", &wanted.i_dont_like, routes::PREFERENCE_MAX)?;
                if wanted != current {
                    planned.push(social_calls::fragrance_preferences_save(&wanted.i_like, &wanted.i_dont_like));
                }
                prefs_before = Some(current);
                wanted_prefs = Some(wanted);
            }
            let about_signature = p.signature_scent.is_some();
            let nick = c.account.own_nick().await.map_err(|e| self.ce(e))?;
            let mut signature_before = None;
            // `Some(None)`: remove it. `Some(Some(slug or p_id))`: set it.
            let mut wanted_signature: Option<Option<String>> = None;
            if let Some(reference) = p.signature_scent.as_deref().map(str::trim) {
                signature_before = c.account.member_profile(&nick, false).await.map_err(|e| self.ce(e))?.signature_scent;
                if reference.is_empty() || matches!(reference.to_lowercase().as_str(), "none" | "clear" | "remove") {
                    if signature_before.is_some() {
                        planned.push(social_calls::signature_scent(None));
                    }
                    wanted_signature = Some(None);
                } else {
                    let (p_id, slug) = c.public.perfume_numeric_id(reference).await.map_err(|e| self.ce(e))?;
                    let same = slug.as_ref().is_some_and(|s| signature_before.as_ref().is_some_and(|b| b.id.eq_ignore_ascii_case(s)));
                    if !same {
                        planned.push(social_calls::signature_scent(Some(p_id)));
                    }
                    wanted_signature = Some(Some(slug.unwrap_or_default()));
                }
            }
            let details = |settings: Vec<Setting>| settings.into_iter().map(|s| setting_out(SettingsPage::PersonalDetails, s)).collect::<Vec<_>>();
            let base = ProfileWriteOutput { public_writes_enabled: public_writes, ..Default::default() };
            if planned.is_empty() {
                let out = ProfileWriteOutput {
                    status: "unchanged".into(),
                    personal_details: details(personal),
                    fragrance_preferences: prefs_before,
                    signature_scent: signature_before,
                    ..base
                };
                return Ok(("Parfumo already holds these values: nothing was sent.".to_string(), out));
            }
            let mut notes = vec!["Gender, country, fragrance preferences and signature scent show on the member's public profile.".to_string()];
            if about_prefs {
                notes.push(OWN_WORDS_NOTE.to_string());
            }
            if dry_run {
                if !public_writes {
                    notes.insert(0, GATE_NOTE.to_string());
                }
                let out = ProfileWriteOutput {
                    dry_run: true,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    personal_details: details(personal),
                    fragrance_preferences: prefs_before,
                    signature_scent: signature_before,
                    notes,
                    ..base
                };
                return Ok((
                    format!("Dry run: nothing was sent to Parfumo. Requests that would be made:\n{}", planned_text(&out.planned_requests)),
                    out,
                ));
            }
            self.send_public(&c, "parfumo_update_profile", &planned).await?;
            let after = self.personal_details(&c).await?;
            let mut wrong: Vec<String> = wanted_settings
                .iter()
                .filter(|(field, value)| after.iter().find(|s| s.field == *field).and_then(|s| s.value.as_deref()) != Some(value.as_str()))
                .map(|(field, _)| field.to_string())
                .collect();
            let prefs_after = if about_prefs { Some(c.account.fragrance_preferences().await.map_err(|e| self.ce(e))?) } else { None };
            if prefs_after != wanted_prefs {
                wrong.push("fragrance preferences".into());
            }
            let signature_after =
                if about_signature { c.account.member_profile(&nick, false).await.map_err(|e| self.ce(e))?.signature_scent } else { None };
            match &wanted_signature {
                Some(None) if signature_after.is_some() => wrong.push("signature scent".into()),
                Some(Some(slug))
                    if signature_after.is_none()
                        || (!slug.is_empty() && signature_after.as_ref().is_some_and(|s| !s.id.eq_ignore_ascii_case(slug))) =>
                {
                    wrong.push("signature scent".into())
                }
                _ => {}
            }
            let status = if wrong.is_empty() {
                "updated"
            } else {
                notes.push(format!("Parfumo accepted the requests, but reading the profile back differs for: {}.", wrong.join(", ")));
                "unverified"
            };
            let out = ProfileWriteOutput {
                executed: true,
                status: status.into(),
                planned_requests: planned,
                personal_details: details(after),
                fragrance_preferences: prefs_after,
                signature_scent: signature_after,
                notes,
                ..base
            };
            let mut text = format!("Status: {status}.");
            for s in &out.personal_details {
                text.push_str(&format!("\n{}", setting_line(s)));
            }
            for n in &out.notes {
                text.push_str(&format!("\nNote: {n}"));
            }
            Ok((text, out))
        })
        .await
    }

    #[tool(
        name = "parfumo_get_settings",
        description = "Read the settings of the user's Parfumo account: notifications and privacy toggles, website options (display, mailbox, shelves, Souk pause), and on request gender, country and time zone (`page: \"personal_details\"`). Each setting comes with its name (`field`), label, current value and choices. Long choice lists (countries, time zones) are returned with `options_of` and `search`. Requires a linked Parfumo account.",
        annotations(title = "Get my settings", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<SettingsOutput>()
    )]
    async fn get_settings(&self, Parameters(p): Parameters<GetSettingsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{:?} options_of={}", p.page, p.options_of.as_deref().unwrap_or("-"));
        self.run_account(&caller, "parfumo_get_settings", &target, |c| async move {
            let options_of = p.options_of.as_deref().map(str::trim).filter(|f| !f.is_empty());
            let pages: Vec<SettingsPage> = match (p.page, options_of) {
                (Some(page), _) => vec![page],
                (None, Some(field)) if routes::PROFILE_FIELDS.contains(&field) => vec![SettingsPage::PersonalDetails],
                (None, _) => vec![SettingsPage::Notifications, SettingsPage::Website],
            };
            let mut settings: Vec<SettingOut> = Vec::new();
            let mut options: Vec<SettingOption> = Vec::new();
            let mut found = options_of.is_none();
            for page in pages {
                for s in c.account.settings(page).await.map_err(|e| self.ce(e))? {
                    if options_of == Some(s.field.as_str()) {
                        found = true;
                        let needle = p.search.as_deref().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
                        options = s.options.iter().filter(|o| needle.as_ref().is_none_or(|n| o.label.to_lowercase().contains(n))).cloned().collect();
                    }
                    settings.push(setting_out(page, s));
                }
            }
            if !found {
                return Err(ToolError::invalid(format!(
                    "No setting named `{}` on the page(s) read. Settings: {}.",
                    options_of.unwrap_or_default(),
                    settings.iter().map(|s| s.field.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
            let keep = paging::fit_prefix(&options, paging::MAX_STRUCTURED_CHARS / 2);
            let cut = keep < options.len();
            options.truncate(keep);
            let mut header = format!("{} setting(s).", settings.len());
            if let Some(field) = options_of {
                header.push_str(&format!(
                    " {} choice(s) of `{field}`{}: {}",
                    options.len(),
                    if cut { " (list cut: narrow it with `search`)" } else { "" },
                    options.iter().take(40).map(|o| format!("{} ({})", o.label, o.value)).collect::<Vec<_>>().join(", ")
                ));
            }
            let text = paging::digest(header, settings.iter().map(setting_line).collect::<Vec<_>>());
            Ok((text, SettingsOutput { settings, options }))
        })
        .await
    }

    #[tool(
        name = "parfumo_update_settings",
        description = "Change one setting of the user's Parfumo account: a notification or privacy toggle, or a website option, by the `field` name parfumo_get_settings returns. Private to the account. Gender, country and time zone go through parfumo_update_profile; the account language is not changed by this server. Parfumo answers the same whether the setting exists or not, so the tool reads the page again and returns the setting as it is after the call. Supports dry_run. Requires a linked Parfumo account.",
        annotations(title = "Update a setting", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<SettingWriteOutput>()
    )]
    async fn update_settings(&self, Parameters(p): Parameters<UpdateSettingsParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let p = &p;
        let target = format!("{}={} dry_run={}", p.field, p.value, p.dry_run);
        self.run_account(&caller, "parfumo_update_settings", &target, |c| async move {
            let field = p.field.trim();
            if routes::PROFILE_FIELDS.contains(&field) {
                return Err(ToolError::invalid(format!("`{field}` shows on the public profile: change it with parfumo_update_profile.")));
            }
            if field == LANGUAGE_FIELD {
                return Err(ToolError::invalid(
                    "The account language is not changed by this server: its tools read the English site. Change it on parfumo.com if needed.",
                ));
            }
            let mut known: Vec<String> = Vec::new();
            let mut hit: Option<(SettingsPage, Setting)> = None;
            for page in [SettingsPage::Notifications, SettingsPage::Website] {
                let settings = c.account.settings(page).await.map_err(|e| self.ce(e))?;
                known.extend(settings.iter().map(|s| s.field.clone()));
                if let Some(s) = settings.into_iter().find(|s| s.field == field) {
                    hit = Some((page, s));
                    break;
                }
            }
            let Some((page, setting)) = hit else {
                return Err(ToolError::invalid(format!(
                    "Parfumo has no setting named `{field}`. Settings: {}.",
                    known.iter().filter(|f| f.as_str() != LANGUAGE_FIELD).cloned().collect::<Vec<_>>().join(", ")
                )));
            };
            let value = if setting.kind == "toggle" { toggle_value(&p.value)?.to_string() } else { pick_option(&setting, &p.value)?.value.clone() };
            if setting.value.as_deref() == Some(value.as_str()) {
                let out = SettingWriteOutput {
                    dry_run: false,
                    executed: false,
                    status: "unchanged".into(),
                    planned_requests: Vec::new(),
                    setting: setting_out(page, setting),
                    notes: Vec::new(),
                };
                return Ok((format!("Parfumo already holds this value: nothing was sent.\n{}", setting_line(&out.setting)), out));
            }
            let planned = vec![social_calls::setting_set(field, &value)];
            if p.dry_run {
                let out = SettingWriteOutput {
                    dry_run: true,
                    executed: false,
                    status: "dry_run".into(),
                    planned_requests: planned,
                    setting: setting_out(page, setting),
                    notes: Vec::new(),
                };
                return Ok((
                    format!(
                        "Dry run: nothing was sent to Parfumo. Request that would be made:\n{}\nCurrent: {}",
                        planned_text(&out.planned_requests),
                        setting_line(&out.setting)
                    ),
                    out,
                ));
            }
            self.send_all(&c, &planned).await?;
            let after = c.account.settings(page).await.map_err(|e| self.ce(e))?.into_iter().find(|s| s.field == field).ok_or_else(|| {
                ToolError::new(ErrorCode::ParserOutdated, format!("The setting `{field}` is no longer on its page after the write."))
            })?;
            let mut notes = Vec::new();
            let status = if after.value.as_deref() == Some(value.as_str()) {
                "updated"
            } else {
                notes.push("Parfumo answered, but reading the page back does not show the new value.".to_string());
                "unverified"
            };
            let out = SettingWriteOutput {
                dry_run: false,
                executed: true,
                status: status.into(),
                planned_requests: planned,
                setting: setting_out(page, after),
                notes,
            };
            Ok((format!("Status: {status}.\n{}", setting_line(&out.setting)), out))
        })
        .await
    }

    #[tool(
        name = "parfumo_session_status",
        description = "Report the state of the link between this server and the user's Parfumo account: whether an account is linked and in which mode, whether the Parfumo session is valid right now, the nickname, and today's remaining quota of each tool with publicly visible writes (daily cap per user). Use it when an account tool fails, or before a series of public writes. Never returns credentials or cookies.",
        annotations(title = "Session status and quotas", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<SessionStatusOutput>()
    )]
    async fn session_status(&self, Parameters(_p): Parameters<NoParams>, caller: Caller) -> Result<CallToolResult, ErrorData> {
        let user = caller.0.clone();
        let started = Instant::now();
        let cfg = &self.state.cfg;
        let sessions = &self.state.sessions;
        let outcome = async {
            let info = sessions.link_info(user.id).map_err(|e| self.session_err(e))?;
            let (mut session, mut nick) = ("not_linked".to_string(), None);
            if info.is_some() {
                session = match sessions.account_client(user.id).await {
                    Ok(client) => match client.own_nick().await {
                        Ok(n) => {
                            nick = Some(n);
                            "valid"
                        }
                        Err(parfumo_client::ClientError::LoginRequired) => "reauth_required",
                        Err(parfumo_client::ClientError::CaptchaRequired) => "captcha_required",
                        Err(_) => "error",
                    },
                    Err(SessionError::NotLinked) => "not_linked",
                    Err(SessionError::ReauthRequired) => "reauth_required",
                    Err(SessionError::CaptchaRequired) => "captcha_required",
                    Err(SessionError::Internal(_)) => "error",
                }
                .to_string();
            }
            let link_url = self.status_link_page(matches!(session.as_str(), "not_linked" | "reauth_required" | "captcha_required"));
            let out = SessionStatusOutput {
                linked: info.is_some(),
                mode: info.as_ref().map(|i| i.mode.clone()),
                session,
                nick,
                linked_at: info.as_ref().map(|i| iso(i.linked_at)),
                last_login_at: info.as_ref().and_then(|i| i.last_login_at).map(iso),
                link_url,
                public_writes_enabled: cfg.public_writes,
                quotas: self.quotas(user.id),
                quotas_reset_in_secs: public_gate::secs_to_reset(),
                write_recipients: cfg.write_recipients.clone(),
                min_request_interval_ms: cfg.user_min_interval.as_millis() as u64,
            };
            let mut lines = vec![match (&out.nick, out.linked) {
                (Some(n), _) => format!("Linked to the Parfumo account {n} ({} mode); session {}.", out.mode.as_deref().unwrap_or("?"), out.session),
                (None, true) => format!(
                    "A Parfumo account is linked ({} mode) but its session is {}: relink {}.",
                    out.mode.as_deref().unwrap_or("?"),
                    out.session,
                    self.link_url()
                ),
                (None, false) => format!("No Parfumo account is linked: the user links one {}.", self.link_url()),
            }];
            lines.push(if out.public_writes_enabled {
                "Publicly visible writes are enabled.".to_string()
            } else {
                "Publicly visible writes are disabled on this server: such tools answer as dry runs.".to_string()
            });
            lines.push(format!(
                "Remaining today (resets in {} s): {}",
                out.quotas_reset_in_secs,
                out.quotas
                    .iter()
                    .map(|q| format!("{} {}/{}", q.tool.trim_start_matches("parfumo_"), q.remaining, q.daily_cap))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            if !out.write_recipients.is_empty() {
                lines.push(format!("Writes to other members are limited to: {}.", out.write_recipients.join(", ")));
            }
            Ok((lines.join("\n"), out))
        }
        .await;
        self.finish(&user, "parfumo_session_status", "session", started, 0, outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(options: &[(&str, &str)]) -> Setting {
        Setting {
            field: "country_id".into(),
            label: "Country".into(),
            section: None,
            kind: "choice".into(),
            value: None,
            options: options.iter().map(|(v, l)| SettingOption { value: v.to_string(), label: l.to_string() }).collect(),
        }
    }

    #[test]
    fn options_are_picked_by_value_label_or_unique_part() {
        let s = choice(&[("41", "Switzerland"), ("42", "Sweden"), ("7", "Swaziland / Eswatini")]);
        assert_eq!(pick_option(&s, "41").unwrap().label, "Switzerland");
        assert_eq!(pick_option(&s, " sweden ").unwrap().value, "42");
        assert_eq!(pick_option(&s, "eswatini").unwrap().value, "7");
        assert!(pick_option(&s, "sw").unwrap_err().message.contains("several"));
        assert!(pick_option(&s, "Atlantis").unwrap_err().message.contains("Choices: Switzerland (41)"));
        assert_eq!((toggle_value("ON").unwrap(), toggle_value("0").unwrap()), ("true", "false"));
        assert!(toggle_value("maybe").is_err());
        assert_eq!((gender_value("Female"), gender_value("non-binary"), gender_value("x")), ("f", "d", "x"));
    }
}
