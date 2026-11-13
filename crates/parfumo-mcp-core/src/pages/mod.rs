//! The pages a person sees, shared by the HTTP server (`/link`, OAuth login) and by the
//! login page of the local stdio server: language, style, and the form that links a
//! Parfumo account. Plain HTML without script; the caller serves it with [`HEADERS`].

pub mod link;

/// Language of the pages: English, French or German. Chosen by `?lang=`, else by the
/// browser's `Accept-Language`, else English. Tool descriptions, errors and logs stay in
/// English.
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

    /// The language asked for explicitly, else the first supported one of the browser's
    /// `Accept-Language` header.
    pub fn choose(asked: Option<&str>, accept_language: Option<&str>) -> Self {
        if let Some(lang) = asked.and_then(Self::from_code) {
            return lang;
        }
        accept_language
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

/// The language links of a page: `href` gives the address of the page in each language.
pub fn lang_switcher(current: Lang, href: impl Fn(Lang) -> String) -> String {
    let links: String = Lang::ALL
        .into_iter()
        .map(|lang| {
            let code = lang.code();
            if lang == current {
                format!("<a href=\"{}\" hreflang=\"{code}\" aria-current=\"true\">{}</a>", esc(&href(lang)), code.to_uppercase())
            } else {
                format!("<a href=\"{}\" hreflang=\"{code}\">{}</a>", esc(&href(lang)), code.to_uppercase())
            }
        })
        .collect();
    format!("<nav class=\"lang\" aria-label=\"Language\">{links}</nav>")
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// A message shown above a form: green when `ok`, red otherwise.
pub fn msg(ok: bool, text: &str) -> String {
    format!("<p class=\"msg{}\">{}</p>", if ok { "" } else { " err" }, esc(text))
}

/// What a form answers to a request sent from another site: `url` is the address of the
/// page to use instead.
pub fn origin_refused(lang: Lang, url: &str) -> String {
    match lang {
        Lang::En => format!("Request refused: open this page at {url} and submit the form from there."),
        Lang::Fr => format!("Requête refusée : ouvrez cette page à l'adresse {url} et envoyez le formulaire depuis celle-ci."),
        Lang::De => format!("Anfrage abgelehnt: Öffne diese Seite unter {url} und sende das Formular von dort ab."),
    }
}

/// Style of the pages: one narrow card, in colours close to those of parfumo.com. No
/// script and no external resource: the content security policy of [`HEADERS`] allows
/// neither.
const PAGE_CSS: &str = r#"
:root{--bg:#f0f1f6;--card:#fff;--ink:#22252e;--muted:#6b7080;--line:#dcdfe8;--accent:#3d6bd6;--accent-ink:#fff;
--ok-bg:#e7f4ec;--ok-ink:#1d5a36;--err-bg:#fcebe9;--err-ink:#9a2318}
@media (prefers-color-scheme:dark){:root{--bg:#14161b;--card:#1d2027;--ink:#eceef4;--muted:#9aa0af;--line:#353945;
--accent:#86a9f4;--accent-ink:#10131a;--ok-bg:#18301f;--ok-ink:#a6dcb8;--err-bg:#3a1f1c;--err-ink:#f3aea5}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:25rem;margin:0 auto;padding:1.25rem 1rem 2.5rem}
.lang{display:flex;justify-content:flex-end;gap:.25rem;margin:0 0 1rem}
.lang a{padding:.2rem .5rem;border-radius:.4rem;font-size:.78rem;font-weight:600;color:var(--muted);text-decoration:none}
.lang a[aria-current]{background:var(--card);color:var(--ink);box-shadow:0 0 0 1px var(--line)}
.card{background:var(--card);border-radius:.9rem;padding:1.5rem 1.4rem;box-shadow:0 1px 2px rgba(20,25,40,.06),0 0 0 1px var(--line)}
h1{margin:0 0 1.1rem;font-size:1.3rem;line-height:1.25}
h2{margin:1.1rem 0 .2rem;font-size:.9rem}
p{margin:0 0 .9rem}
label{display:block;margin:.9rem 0 .3rem;font-size:.86rem;font-weight:600}
input[type=text],input[type=password],textarea{width:100%;padding:.6rem .7rem;border:1px solid var(--line);border-radius:.55rem;
background:var(--card);color:var(--ink);font:inherit}
textarea{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.85rem;resize:vertical}
input:focus-visible,textarea:focus-visible,button:focus-visible,summary:focus-visible,a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.hint{margin:.3rem 0 0;font-size:.8rem;color:var(--muted)}
.check{display:flex;gap:.6rem;align-items:flex-start;margin:1.1rem 0 0;font-weight:400;font-size:.86rem;cursor:pointer}
.check input{margin:.2rem 0 0;accent-color:var(--accent)}
button{display:block;width:100%;margin:1.1rem 0 0;padding:.7rem 1rem;border:0;border-radius:.55rem;background:var(--accent);
color:var(--accent-ink);font:inherit;font-weight:600;cursor:pointer}
button.quiet{margin-top:.7rem;background:transparent;color:var(--ink);box-shadow:inset 0 0 0 1px var(--line);font-weight:500;font-size:.9rem}
button:hover{filter:brightness(1.07)}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:.6rem}
details{margin:1.3rem 0 0;padding-top:1rem;border-top:1px solid var(--line)}
summary{font-size:.86rem;color:var(--muted);cursor:pointer}
.msg{margin:0 0 1rem;padding:.7rem .9rem;border-radius:.55rem;background:var(--ok-bg);color:var(--ok-ink);font-size:.92rem}
.msg.err{background:var(--err-bg);color:var(--err-ink)}
.lead{color:var(--muted);font-size:.92rem}
.lead strong{color:var(--ink)}
.to{margin:0 0 .9rem;padding:.6rem .7rem;border-radius:.55rem;background:var(--bg);font-size:.82rem;overflow-wrap:anywhere}
.to span{display:block;color:var(--muted)}
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.92em}
.foot{margin:1.1rem .4rem 0;font-size:.76rem;color:var(--muted);text-align:center}
"#;

/// A whole page: the language links, then one card holding the title and `body`. A
/// paragraph of class `foot` in `body` is shown under the card.
pub fn html(lang: Lang, title: &str, switcher: &str, body: &str) -> String {
    let (card, foot) = match body.rfind("<p class=\"foot\">") {
        Some(at) => body.split_at(at),
        None => (body, ""),
    };
    format!(
        "<!doctype html><html lang=\"{code}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"light dark\"><title>{t} · Parfumo MCP</title>\
         <style>{PAGE_CSS}</style></head><body><main>{switcher}<div class=\"card\"><h1>{t}</h1>{card}</div>{foot}</main></body></html>",
        code = lang.code(),
        t = esc(title)
    )
}

/// Response headers of every page. Nothing leaks to other sites, and the forms keep their
/// `Origin` header, which the servers check: with `Referrer-Policy: no-referrer` a browser
/// would send `Origin: null`.
pub const HEADERS: [(&str, &str); 4] = [
    ("cache-control", "no-store"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "same-origin"),
    ("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_asked_language_wins_then_the_browser_then_english() {
        assert_eq!(Lang::choose(Some("de"), Some("fr-FR,fr;q=0.9")), Lang::De);
        assert_eq!(Lang::choose(None, Some("fr-FR,fr;q=0.9,en;q=0.8")), Lang::Fr);
        assert_eq!(Lang::choose(None, Some("es-ES,de-AT;q=0.8,en;q=0.5")), Lang::De, "first supported one");
        assert_eq!(Lang::choose(Some("xx"), Some("it")), Lang::En);
        assert_eq!(Lang::choose(None, None), Lang::En);
        assert_eq!(Lang::from_code("FR-ca"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("french"), None);
    }

    #[test]
    fn a_page_puts_the_footer_under_the_card_and_escapes_its_title() {
        let page = html(Lang::Fr, "A <b>", "", "<p>body</p><p class=\"foot\">note</p>");
        assert!(page.contains("<html lang=\"fr\">") && page.contains("<h1>A &lt;b&gt;</h1><p>body</p></div><p class=\"foot\">note</p>"), "{page}");
        assert!(!page.contains("<script"), "no script: the content security policy forbids it");
    }
}
