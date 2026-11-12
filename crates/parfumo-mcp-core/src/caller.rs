//! Who a tool call belongs to, whatever the transport.

use http::request::Parts;
use rmcp::{
    ErrorData,
    handler::server::{common::FromContextPart, tool::ToolCallContext},
};

use crate::{db::User, state::AuthUser, tools::ParfumoTools};

/// The user of this server a tool call belongs to. On the HTTP server it is the user the
/// bearer token of the request authenticated; on the stdio server, the single local user.
#[derive(Debug, Clone)]
pub struct Caller(pub User);

impl FromContextPart<ToolCallContext<'_, ParfumoTools>> for Caller {
    fn from_context_part(context: &mut ToolCallContext<'_, ParfumoTools>) -> Result<Self, ErrorData> {
        if let Some(user) = &context.service.local {
            return Ok(Self(user.clone()));
        }
        context
            .request_context
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<AuthUser>())
            .map(|AuthUser(user)| Self(user.clone()))
            .ok_or_else(|| ErrorData::invalid_request("unauthenticated request", None))
    }
}
