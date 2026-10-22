//! `GET /api/state`: the product state a page reads once its synchronization
//! channel is open.
//!
//! The lists only a settings pane shows are not in it; each has its own route.
//! An account still to choose its password is refused here as it is on every
//! route but the few that open before the choice, and reads the state once the
//! channel tells it the choice landed. The answer names the version of the
//! state it was read at, which the page follows on the channel.

use axum::Json;
use axum::extract::State;
use demi_web_api_protocol::state::ProductState;

use super::AppState;
use super::error::ApiError;
use super::gate::AuthUser;

pub(super) async fn read(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<ProductState>, ApiError> {
    let id = user.id.clone();
    let product = state
        .shards
        .of(&id)
        .call(move |shard, _| async move {
            shard
                .product_state(user)
                .await
                .map_err(|error| ApiError::internal(&error))
        })
        .await??;
    Ok(Json(product))
}
