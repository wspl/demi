//! A core's routes: what a page reads and changes of its core, and how a
//! runner reaches the core, over HTTP and the page connection.
//!
//! Two ways in mount the same routes:
//!
//! ```text
//! the server: /api, behind its session gate, beside its own routes
//!   (accounts, sessions, users, upgrades, shares, lending, pairing)
//! a program's process: /cores/{id}/api, for the program that opened it
//! ```
//!
//! A route takes its caller from the request, as the mount resolved it
//! ([`caller`]), reaches the caller's core through the pool, and its store
//! through the lookup the vault already uses. The mount decides who calls;
//! a route decides only what the caller's core does.

// The text of the pages the routes render, one catalogue per language.
rust_i18n::i18n!("locales", fallback = "en");

mod accounts;
pub mod api;
mod attachments;
pub mod blobs;
mod channel;
pub mod blocks;
pub mod body;
pub mod caller;
pub mod connection;
pub mod content;
pub mod conversations;
mod cost;
mod credentials;
pub mod devices;
mod direct;
mod drafts;
pub mod error;
mod file_watch;
mod files;
pub mod history;
mod hosts;
pub mod html_view;
mod instructions;
mod models;
mod oauth;
pub mod page_client;
mod panel;
pub mod peer;
mod permissions;
pub mod plugins;
mod port_forwards;
mod projects;
mod provider_cli;
pub mod providers;
pub mod query;
pub mod request_id;
pub mod revalidate;
mod saved;
pub mod schedules;
mod search;
mod preferences;
mod side_questions;
mod sidebar;
pub mod state;
mod streams;
mod subagents;
mod suggested_replies;
pub mod sync;
pub mod transfer;
mod usage;
mod workspaces;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, patch, post, put};

use crate::api::Api;

/// The core's routes a caller reaches, for the mount to put behind what
/// resolves the caller. The product state's read and the page connection
/// are the mount's to route, beside its own parts of them
/// ([`state::read`], [`connection::connect`]).
pub fn routes() -> Router<Api> {
    Router::new()
        .route("/settings/instructions", get(instructions::read))
        .route("/settings/subagents", get(subagents::read))
        .route(
            "/settings/preferences",
            get(preferences::preferences).patch(preferences::patch_preferences),
        )
        .route(
            "/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(attachments::BODY_LIMIT)),
        )
        .route("/blobs/{sha256}", get(blobs::blob))
        .route("/models/refresh", post(models::refresh))
        .route("/models/search", get(models::search))
        .route("/providers", get(providers::list).post(providers::create))
        .route("/providers/catalog", get(providers::catalog))
        .route("/providers/order", put(providers::order))
        .route("/providers/subscription-login", post(accounts::start_login))
        .route(
            "/providers/subscription-login/{id}",
            get(accounts::login_state).delete(accounts::cancel_login),
        )
        .route("/providers/subscription-login/{id}/code", post(accounts::submit_code))
        .route("/providers/{id}", patch(providers::update).delete(providers::delete))
        .route("/providers/{id}/status", get(providers::status))
        .route("/providers/{id}/quota", post(providers::quota))
        .route("/providers/{id}/test", post(providers::test))
        .route("/providers/{id}/cli", get(provider_cli::read))
        .route("/providers/{id}/cli/install", post(provider_cli::install))
        .route("/providers/{id}/accounts", get(accounts::list))
        .route("/providers/{id}/accounts/active", put(accounts::activate))
        .route("/providers/{id}/accounts/order", put(accounts::reorder))
        .route("/providers/{id}/accounts/login", post(accounts::login_into))
        .route("/providers/{id}/accounts/{credential}", delete(accounts::remove))
        .route("/search", get(search::search))
        .route("/usage", get(usage::totals))
        .route("/conversations", get(conversations::list).post(conversations::create))
        .route("/conversations/batch", post(conversations::batch))
        .route(
            "/conversations/{id}",
            get(conversations::detail)
                .patch(conversations::patch)
                .delete(conversations::delete),
        )
        .route("/conversations/{id}/fork", post(conversations::fork))
        .route("/conversations/{id}/side-questions", post(side_questions::ask))
        .route(
            "/conversations/{id}/side-questions/{side}",
            patch(side_questions::fold).delete(side_questions::delete),
        )
        .route(
            "/conversations/{id}/side-questions/{side}/messages",
            post(side_questions::follow_up),
        )
        .route("/conversations/{id}/side-questions/{side}/retry", post(side_questions::retry))
        .route("/conversations/{id}/side-questions/{side}/stop", post(side_questions::stop))
        .route("/conversations/{id}/workflows", get(conversations::workflows))
        .route("/conversations/{id}/agents/{node}/transcript", get(history::transcript))
        .route("/conversations/{id}/agents/{node}/blocks/{block}", get(history::block))
        .route("/conversations/{id}/commands/{command}", get(history::command))
        .route("/conversations/{id}/read", post(conversations::read))
        .route(
            "/conversations/{id}/attachments/{attachment}/file",
            get(conversations::attachment_file),
        )
        .route("/conversations/{id}/cost/recent", get(cost::recent))
        .route("/conversations/{id}/search", get(search::search_in))
        .route("/conversations/{id}/panel/changes", post(panel::change))
        .route("/conversations/{id}/suggested-replies/used", post(suggested_replies::used))
        .route("/conversations/{id}/draft", get(drafts::read).put(drafts::save))
        .route("/conversations/{id}/saved", get(saved::list).post(saved::save))
        .route("/conversations/{id}/saved/from-draft", post(saved::from_draft))
        .route(
            "/conversations/{id}/saved/{message}",
            get(saved::read).put(saved::write).delete(saved::remove),
        )
        .route(
            "/conversations/{id}/permissions/requests/{request}",
            post(permissions::decide),
        )
        .route(
            "/conversations/{id}/plugins/{plugin}/calls/{method}",
            post(plugins::conversation_call),
        )
        .route("/conversations/{id}/plugins/{plugin}/state", get(plugins::conversation_state))
        .route("/plugins/{plugin}/calls/{method}", post(plugins::user_call))
        .route("/plugins/{plugin}/blocks/{tag}/{id}", get(blocks::document))
        .route("/reply-blocks", post(blocks::facts))
        .route("/plugins/{plugin}/oauth/callback", get(oauth::callback))
        .route("/plugins/{plugin}", put(plugins::switch))
        .route("/plugins/{plugin}/features/{feature}", put(plugins::switch_feature))
        .route("/instructions", put(instructions::replace))
        .route(
            "/conversations/{id}/schedules/{schedule}",
            patch(schedules::change).delete(schedules::delete),
        )
        .route("/port-forwards", get(port_forwards::list).post(port_forwards::create))
        .route("/port-forwards/{id}", patch(port_forwards::patch).delete(port_forwards::delete))
        .route("/port-forwards/{id}/retry", post(port_forwards::retry))
        .route("/credentials", get(credentials::list).post(credentials::create))
        .route("/credentials/{id}", patch(credentials::change).delete(credentials::delete))
        .route("/credentials/{id}/fields/{field}/reveal", post(credentials::reveal))
        .route("/subagents", put(subagents::switch))
        .route("/subagents/profiles", post(subagents::create))
        .route("/subagents/profiles/{id}", patch(subagents::patch).delete(subagents::delete))
        .route("/conversations/{id}/reload", post(plugins::reload))
        .route("/sidebar/reorder", post(sidebar::reorder))
        .route("/projects", get(projects::list).post(projects::create))
        .route("/projects/{id}", patch(projects::change).delete(projects::delete))
        .route("/projects/{id}/saved-scope", put(projects::switch_saved_scope))
        .route("/workspaces", get(workspaces::list).post(workspaces::create))
        .route("/workspaces/order", put(workspaces::order))
        .route("/workspaces/{id}", patch(workspaces::change).delete(workspaces::delete))
        .route("/workspaces/{id}/moves", post(workspaces::moves))
        .route("/devices", get(devices::list))
        .route("/devices/{id}", delete(devices::revoke).patch(devices::change))
        .route("/devices/{id}/fs", get(devices::browse).post(devices::make_directory))
        .route("/devices/{id}/workflows", get(devices::workflows))
        .route(
            "/conversations/{id}/fs",
            get(files::list).post(files::make_directory).delete(files::remove),
        )
        .route("/conversations/{id}/fs/lists", post(files::lists))
        .route("/conversations/{id}/fs/search", post(files::search))
        .route("/conversations/{id}/fs/html", post(html_view::grant))
        .route("/conversations/{id}/fs/file", get(files::text))
        .route("/conversations/{id}/fs/raw", get(files::raw).put(files::upload))
        .route("/conversations/{id}/hosts/{device}", delete(hosts::detach))
        .route(
            "/conversations/{id}/hosts/{device}/fs",
            get(files::list_on_host).post(files::make_directory_on_host),
        )
        .route("/conversations/{id}/changes", get(files::changes))
        .route("/conversations/{id}/changes/file", get(files::changed_file))
        .route("/conversations/{id}/changes/raw", get(files::committed))
        .route("/conversations/{id}/changes/branches", get(files::branches))
        .route("/conversations/{id}/changes/repositories", post(files::repositories))
        .route("/conversations/{id}/changes/log", get(files::log))
        .route("/conversations/{id}/changes/commit", get(files::commit))
        // The reads' caching (`revalidate.rs`): the routes' answers only.
        .layer(middleware::from_fn(revalidate::reads))
}

/// The routes a page's frames load with no caller, whose path carries their
/// only key: the HTML files the File view shows, by a token, and the files a
/// plugin's page loads.
pub fn public_routes() -> Router<Api> {
    Router::new()
        .route("/html/{token}/", get(html_view::serve_root))
        .route("/html/{token}/{*place}", get(html_view::serve))
        .layer(middleware::map_response(html_view::headers))
        .route("/plugins/{plugin}/files/{name}", get(plugins::file))
}
