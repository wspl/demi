//! `GET /api/update`, and what the listener answers before the backend
//! serves.
//!
//! The backend listens before it opens its databases, and its update
//! screen's mode listens without them. Until it serves, it answers:
//!
//! - `GET /api/update`: the steps of the update, with the conversations and
//!   bytes done of how many and the time left; it names no conversation.
//!   Once it serves, it answers that nothing runs, or that the backend stops
//!   next for an update of the server.
//! - The web app's files, so a page loads and shows the progress.
//! - 503 to every other request: `updating` while an update runs, so a page
//!   loaded then opens on its screen, `backend_starting` otherwise, which a
//!   page and a runner take as a backend away for a moment.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use demi_backend_database::update::Update;
use demi_backend_runners::{native, public_url::PublicUrl};
use demi_web_api_protocol::state::UpdateState;

use super::assets;
use super::error::ApiError;

/// `GET /api/update`.
pub(crate) async fn read(State(update): State<Arc<Update>>) -> Json<UpdateState> {
    Json(update.state())
}

/// The routes before the backend serves.
pub(crate) fn routes(update: Arc<Update>, web_directory: Option<PathBuf>, public: PublicUrl) -> Router {
    let mut app = Router::new().route("/api/update", get(read));
    for path in [
        "/api",
        "/api/{*rest}",
        "/hooks/{*rest}",
        "/install.sh",
        "/install.ps1",
        "/runner-artifacts/{*rest}",
        &format!("{}/{{*rest}}", native::ROUTE),
    ] {
        app = app.route(path, any(not_yet));
    }
    let app = match web_directory {
        Some(directory) => assets::serve(app, directory, public),
        None => app.fallback(not_yet),
    };
    app.with_state(update)
}

/// The 503 of a backend that does not serve yet.
async fn not_yet(State(update): State<Arc<Update>>) -> Response {
    match update.state() {
        UpdateState::Updating { .. } => ApiError::updating(),
        UpdateState::None => ApiError::backend_starting(),
    }
    .into_response()
}
