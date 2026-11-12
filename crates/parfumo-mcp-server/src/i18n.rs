//! Language of the two pages a person sees (`/link` and the OAuth login): English, French
//! or German. Chosen by `?lang=`, else by the browser's `Accept-Language`, else English.
//! Tool descriptions, errors and logs stay in English.

use axum::http::{HeaderMap, header};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Fr,
    De,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Fr, Lang::De];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| code == l.code() || code.starts_with(&format!("{}-", l.code())))
    }

    /// The language asked for explicitly, else the first supported one the browser prefers.
    pub fn choose(asked: Option<&str>, headers: &HeaderMap) -> Self {
        if let Some(lang) = asked.and_then(Self::from_code) {
            return lang;
        }
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|accept| accept.split(',').filter_map(|part| Self::from_code(part.split(';').next().unwrap_or(""))).next())
            .unwrap_or_default()
    }

    /// One of three texts.
    pub fn pick(self, en: &'static str, fr: &'static str, de: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Fr => fr,
            Lang::De => de,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT_LANGUAGE, value.parse().unwrap());
        headers
    }

    #[test]
    fn the_asked_language_wins_then_the_browser_then_english() {
        assert_eq!(Lang::choose(Some("de"), &accept("fr-FR,fr;q=0.9")), Lang::De);
        assert_eq!(Lang::choose(None, &accept("fr-FR,fr;q=0.9,en;q=0.8")), Lang::Fr);
        assert_eq!(Lang::choose(None, &accept("es-ES,de-AT;q=0.8,en;q=0.5")), Lang::De, "first supported one");
        assert_eq!(Lang::choose(Some("xx"), &accept("it")), Lang::En);
        assert_eq!(Lang::choose(None, &HeaderMap::new()), Lang::En);
        assert_eq!(Lang::from_code("FR-ca"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("french"), None);
    }
}
