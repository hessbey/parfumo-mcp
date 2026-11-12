//! Router assembly and server lifecycle.

use std::{sync::Arc, time::Duration};

use axum::{
    Router, middleware,
    routing::{get, post},
};
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use tokio_util::sync::CancellationToken;

use parfumo_mcp_core::{config::Config, crypto::MasterKey, db::Db, state::AppState, tools::ParfumoTools};

use crate::{
    config::ServerConfig,
    enroll, oauth,
    web::{self, ServerState},
};

pub fn state(cfg: Config, http: ServerConfig, db: Db, key: MasterKey) -> ServerState {
    ServerState { core: AppState::new(cfg, db, key), http: Arc::new(http) }
}

/// Origin of the public URL with an explicit port, as rmcp's Origin validation expects.
fn public_origin(cfg: &ServerConfig) -> String {
    let u = url::Url::parse(&cfg.public_url).expect("validated at startup");
    format!("{}://{}:{}", u.scheme(), u.host_str().unwrap_or_default(), u.port_or_known_default().unwrap_or(443))
}

pub fn router(state: ServerState, shutdown: CancellationToken) -> Router {
    let cfg = &state.http;
    // Stateless transport (2026-07-28 removes sessions); older protocol versions that still
    // use `initialize` are served statelessly too. DNS-rebinding and Origin checks are on.
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_cancellation_token(shutdown)
        .with_allowed_hosts([cfg.public_authority(), "localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()])
        .with_allowed_origins([public_origin(cfg)]);
    let (tools_state, link_page) = (state.core.clone(), cfg.link_page());
    let mcp = StreamableHttpService::new(
        move || Ok(ParfumoTools::remote(tools_state.clone(), link_page.clone())),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    let protected = Router::new().nest_service("/mcp", mcp).layer(middleware::from_fn_with_state(state.clone(), web::require_bearer));
    Router::new()
        .route("/healthz", get(web::healthz))
        .route("/.well-known/oauth-protected-resource", get(oauth::protected_resource_metadata))
        .route("/.well-known/oauth-protected-resource/mcp", get(oauth::protected_resource_metadata))
        .route("/.well-known/oauth-authorization-server", get(oauth::authorization_server_metadata))
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/authorize", get(oauth::authorize_get).post(oauth::authorize_post))
        .route("/oauth/token", post(oauth::token))
        .route("/link", get(enroll::get).post(enroll::post))
        .with_state(state)
        .merge(protected)
}

pub async fn serve(cfg: Config, http: ServerConfig, db: Db, key: MasterKey) -> Result<(), String> {
    // JSON logs. rmcp logs every stateless request at INFO: keep it at WARN.
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let targets = tracing_subscriber::filter::Targets::new().with_default(tracing::Level::INFO).with_target("rmcp", tracing::Level::WARN);
    tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().json()).with(targets).init();
    db.purge().map_err(|e| e.to_string())?;
    let bind = http.bind;
    if !http.public_url.starts_with("https://") {
        tracing::warn!(public_url = %http.public_url, "PARFUMO_MCP_PUBLIC_URL is not https: acceptable for local tests only");
    }
    let state = state(cfg, http, db.clone(), key);
    let shutdown = CancellationToken::new();
    let app = router(state.clone(), shutdown.clone());

    let purge_db = db.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = purge_db.purge() {
                tracing::warn!(error = %e, "database purge failed");
            }
        }
    });

    match state.http.canary_hour {
        Some(hour) => {
            tracing::info!(hour_utc = hour, account_user = ?state.http.canary_user, "daily canary scheduled");
            tokio::spawn(crate::canary::daily(state.clone(), hour));
        }
        None => tracing::warn!("the daily canary is switched off (PARFUMO_MCP_CANARY_HOUR=off)"),
    }

    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| format!("cannot listen on {bind}: {e}"))?;
    tracing::info!(%bind, version = env!("CARGO_PKG_VERSION"), "parfumo-mcp-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("shutting down");
            shutdown.cancel();
        })
        .await
        .map_err(|e| e.to_string())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
