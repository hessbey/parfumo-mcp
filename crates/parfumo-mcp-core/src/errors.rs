//! Normalised tool errors (cahier des charges, section 8).

use parfumo_client::ClientError;
use rmcp::model::{CallToolResult, ContentBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotLinked,
    ReauthRequired,
    CaptchaRequired,
    RateLimited,
    NotFound,
    ParserOutdated,
    UpstreamError,
    /// Not in the original table: an argument the tool cannot use (bad cursor, foreign URL…).
    InvalidInput,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotLinked => "not_linked",
            Self::ReauthRequired => "reauth_required",
            Self::CaptchaRequired => "captcha_required",
            Self::RateLimited => "rate_limited",
            Self::NotFound => "not_found",
            Self::ParserOutdated => "parser_outdated",
            Self::UpstreamError => "upstream_error",
            Self::InvalidInput => "invalid_input",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: String,
    pub retry_after_secs: Option<u64>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), retry_after_secs: None }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    /// `link_url` tells how the user fixes account problems, as the end of a sentence: "at `<the enrolment page>`", or "by setting `<variables>`" on the stdio server.
    pub fn from_client(e: ClientError, link_url: &str) -> Self {
        match e {
            ClientError::NotFound(what) => Self::new(
                ErrorCode::NotFound,
                format!("Not found on Parfumo: {what}. Check the identifier, or use parfumo_search_perfumes to find it."),
            ),
            ClientError::RateLimited { retry_after_secs } => Self {
                code: ErrorCode::RateLimited,
                message: match retry_after_secs {
                    Some(s) => format!("Parfumo or this server is rate limiting requests. Wait {s} seconds before retrying."),
                    None => "Parfumo or this server is rate limiting requests. Wait about a minute before retrying.".into(),
                },
                retry_after_secs: Some(retry_after_secs.unwrap_or(60)),
            },
            ClientError::CaptchaRequired => Self::new(
                ErrorCode::CaptchaRequired,
                format!(
                    "Parfumo asks for a human verification that this server never solves. The user must log in to Parfumo \
                     in their own browser and import the session cookie {link_url}. Do not retry in a loop."
                ),
            ),
            ClientError::ParserOutdated(what) => Self::new(
                ErrorCode::ParserOutdated,
                format!(
                    "The Parfumo page structure changed and this tool must be updated ({what}). Retrying will not help; parfumo_fetch_page may still read the page as text."
                ),
            ),
            ClientError::Upstream(what) => {
                Self::new(ErrorCode::UpstreamError, format!("Parfumo is unavailable or returned an error ({what}). Retry later."))
            }
            ClientError::InvalidCredentials | ClientError::LoginRequired => Self::new(
                ErrorCode::ReauthRequired,
                format!("The Parfumo session cannot be renewed. The user must link the account again {link_url}."),
            ),
            ClientError::InvalidInput(what) => Self::invalid(what),
        }
    }

    /// Tool-level error (`isError: true`) with a message telling the assistant what to do.
    pub fn into_result(self) -> CallToolResult {
        let body = serde_json::json!({
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
                "retry_after_secs": self.retry_after_secs,
            }
        });
        CallToolResult::error(vec![ContentBlock::text(format!("{}: {}\n{}", self.code.as_str(), self.message, body))])
    }
}

impl ToolError {
    /// Account-session failures. Used by the account tools of the next milestones; the
    /// ten database tools of jalon 1 never need a linked account.
    pub fn from_session(e: crate::sessions::SessionError, link_url: &str) -> Self {
        use crate::sessions::SessionError as S;
        match e {
            S::NotLinked => Self::new(
                ErrorCode::NotLinked,
                format!(
                    "No Parfumo account is linked for this user. The user must link one {link_url} (never ask for credentials in the conversation)."
                ),
            ),
            S::ReauthRequired => Self::from_client(ClientError::LoginRequired, link_url),
            S::CaptchaRequired => Self::from_client(ClientError::CaptchaRequired, link_url),
            S::Internal(m) => Self::new(ErrorCode::UpstreamError, m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionError;

    const LINK: &str = "at https://mcp.example.com/link";

    #[test]
    fn every_error_has_its_code_and_tells_what_to_do() {
        let cases = [
            (ToolError::from_client(ClientError::NotFound("perfume X".into()), LINK), "not_found", "parfumo_search_perfumes"),
            (ToolError::from_client(ClientError::RateLimited { retry_after_secs: Some(30) }, LINK), "rate_limited", "30 seconds"),
            (ToolError::from_client(ClientError::CaptchaRequired, LINK), "captcha_required", LINK),
            (ToolError::from_client(ClientError::ParserOutdated("rating block".into()), LINK), "parser_outdated", "must be updated"),
            (ToolError::from_client(ClientError::Upstream("HTTP 503".into()), LINK), "upstream_error", "Retry later"),
            (ToolError::from_client(ClientError::InvalidCredentials, LINK), "reauth_required", LINK),
            (ToolError::from_session(SessionError::NotLinked, LINK), "not_linked", LINK),
            (ToolError::from_session(SessionError::ReauthRequired, LINK), "reauth_required", LINK),
            (ToolError::from_session(SessionError::CaptchaRequired, LINK), "captcha_required", LINK),
            (ToolError::invalid("bad cursor"), "invalid_input", "bad cursor"),
        ];
        for (err, code, hint) in cases {
            assert_eq!(err.code.as_str(), code);
            assert!(err.message.contains(hint), "{code}: {}", err.message);
            let result = err.into_result();
            assert_eq!(result.is_error, Some(true));
            assert!(result.structured_content.is_none(), "an error must not pose as structured output");
            let text = result.content[0].as_text().unwrap().text.clone();
            assert!(text.starts_with(&format!("{code}: ")), "{text}");
        }
        assert_eq!(ToolError::from_client(ClientError::RateLimited { retry_after_secs: None }, LINK).retry_after_secs, Some(60));
    }
}
