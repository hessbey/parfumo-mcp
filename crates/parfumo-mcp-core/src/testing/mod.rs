//! Test support, without network: a configuration for tests, a stand-in for parfumo.com
//! ([`site`]) and [`TestApp`], which calls the tools in process, one MCP session per user,
//! over an in-memory pipe. No HTTP is involved: the HTTP server has its own tests.
//!
//! Compiled for the tests of this crate and, with the `test-util` feature, for the tests of
//! the binaries built on it.

pub mod site;

use std::{collections::HashMap, time::Duration};

use rmcp::ServiceExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

use crate::{
    config::{Config, WriteCaps},
    crypto,
    db::Db,
    state::AppState,
    tools::ParfumoTools,
};

pub fn test_config(parfumo_base_url: &str) -> Config {
    Config {
        db_path: ":memory:".into(),
        parfumo_base_url: parfumo_base_url.trim_end_matches('/').to_string(),
        user_min_interval: Duration::from_millis(1),
        user_jitter: Duration::ZERO,
        // 0 disables the global limiter (it has its own test in parfumo-client).
        global_rps: 0,
        public_writes: false,
        public_write_caps: WriteCaps::default(),
        write_recipients: Vec::new(),
    }
}

/// Links the Parfumo account `name` of the stand-in site to a user, in credentials mode.
pub async fn link_account(state: &AppState, user_id: i64, name: &str) {
    state.sessions.link_credentials(user_id, name, &format!("pw-{name}")).await.unwrap();
}

/// A user whose Parfumo account `name` is linked in credentials mode: `(user_id, token)`.
pub async fn linked(app: &TestApp, name: &str) -> (i64, String) {
    let (id, token) = app.add_user(name);
    link_account(&app.state, id, name).await;
    (id, token)
}

pub fn ok(result: &Value) -> &Value {
    assert_ne!(result["isError"], true, "{result}");
    &result["structuredContent"]
}

pub fn error_code(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().split(':').next().unwrap().to_string()
}

/// One MCP session of one user, as JSON-RPC lines over an in-memory pipe.
struct Session {
    lines: tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>,
    write: WriteHalf<DuplexStream>,
    next_id: u64,
}

impl Session {
    async fn open(tools: ParfumoTools) -> Self {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            if let Ok(service) = tools.serve(theirs).await {
                let _ = service.waiting().await;
            }
        });
        let (read, write) = tokio::io::split(ours);
        let mut session = Self { lines: BufReader::new(read).lines(), write, next_id: 0 };
        let init = session
            .request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}))
            .await;
        assert!(init["result"]["capabilities"]["tools"].is_object(), "{init}");
        session.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        session
    }

    async fn send(&mut self, message: &Value) {
        self.write.write_all(format!("{message}\n").as_bytes()).await.unwrap();
        self.write.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        loop {
            let line = self.lines.next_line().await.unwrap().expect("the tools closed the session");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC ({e}): {line}"));
            if message["id"] == id {
                return message;
            }
        }
    }
}

/// The tools on a fresh state, called in process. A user is designated by the bearer token
/// [`TestApp::add_user`] returned, as on the HTTP server.
pub struct TestApp {
    pub state: AppState,
    sessions: tokio::sync::Mutex<HashMap<String, Session>>,
}

impl TestApp {
    pub fn new(parfumo_base_url: &str) -> Self {
        Self::with_config(test_config(parfumo_base_url))
    }

    pub fn with_config(cfg: Config) -> Self {
        Self::with_db(cfg, Db::open_in_memory().unwrap())
    }

    /// On an existing database (a restart keeps users and daily counts).
    pub fn with_db(cfg: Config, db: Db) -> Self {
        Self { state: AppState::new(cfg, db, crypto::MasterKey::random()), sessions: Default::default() }
    }

    /// Creates a user; returns `(user_id, access key)`.
    pub fn add_user(&self, name: &str) -> (i64, String) {
        let key = crypto::new_secret("pmcp_");
        let id = self.state.db.add_user(name, &crypto::token_hash(&key)).unwrap();
        (id, key)
    }

    async fn request(&self, bearer: &str, method: &str, params: Value) -> Value {
        let mut sessions = self.sessions.lock().await;
        if !sessions.contains_key(bearer) {
            let user = self.state.db.user_by_bearer_hash(&crypto::token_hash(bearer)).unwrap().expect("unknown bearer token");
            sessions
                .insert(bearer.to_string(), Session::open(ParfumoTools::local(self.state.clone(), user, crate::tools::LinkHow::Environment)).await);
        }
        sessions.get_mut(bearer).unwrap().request(method, params).await
    }

    /// The `result` of `tools/call`: `content`, `structuredContent`, `isError`.
    pub async fn call_tool(&self, bearer: &str, tool: &str, args: Value) -> Value {
        let message = self.request(bearer, "tools/call", json!({"name": tool, "arguments": args})).await;
        assert!(message.get("error").is_none(), "{message}");
        message["result"].clone()
    }

    /// The tools `tools/list` returns, with schemas and annotations.
    pub async fn list_tools(&self, bearer: &str) -> Vec<Value> {
        let message = self.request(bearer, "tools/list", json!({})).await;
        message["result"]["tools"].as_array().unwrap_or_else(|| panic!("tools array: {message}")).clone()
    }
}
