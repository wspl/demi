//! `GET /api/state`: the product state a page reads once its synchronization
//! channel is open, the user's core's with the server's parts beside it
//! (`crates/backend-user-shard/src/page_state.rs`).
//!
//! An account still to choose its password is refused here as it is on every
//! route but the few that open before the choice, and reads the state once the
//! channel tells it the choice landed.

use axum::Json;
use axum::extract::State;
use demi_backend_user_shard::page_state::product_state;
use demi_core_api::error::ApiError;
use demi_web_api_protocol::state::ProductState;

use super::AppState;
use super::gate::AuthUser;

pub(super) async fn read(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<ProductState>, ApiError> {
    let id = user.id.clone();
    let product = state
        .api.shards
        .of(&id)
        .call(move |core, _| async move {
            product_state(&core, user)
                .await
                .map_err(|error| ApiError::internal(&error))
        })
        .await??;
    Ok(Json(product))
}
