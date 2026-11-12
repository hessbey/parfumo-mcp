//! What every tool call works with: configuration, database, Parfumo sessions.

use std::sync::Arc;

use crate::{
    config::Config,
    crypto::MasterKey,
    db::{Db, User},
    sessions::Sessions,
};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Db,
    pub sessions: Arc<Sessions>,
}

impl AppState {
    pub fn new(cfg: Config, db: Db, key: MasterKey) -> Self {
        let sessions = Arc::new(Sessions::new(cfg.clone(), db.clone(), key));
        Self { cfg: Arc::new(cfg), db, sessions }
    }
}

/// The authenticated user of an HTTP request. The HTTP server stores it in the request
/// extensions once the bearer token is verified; [`crate::caller::Caller`] reads it back.
#[derive(Debug, Clone)]
pub struct AuthUser(pub User);
