//! `POST /api/devices/claim`: a user pairs the runner waiting under the code
//! it printed.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_runners::claims::ClaimRefusal;
use demi_core_api::body::JsonBody;
use demi_core_api::error::ApiError;
use demi_web_api_protocol::devices::{Claim, DeviceAnswer};
use demi_web_api_protocol::error::ErrorCode;

use super::AppState;
use super::gate::AuthUser;

/// Pairs the runner waiting under the code with the caller: the device is
/// made, its token goes to that runner alone, and the runner's socket moves
/// into the caller's core.
pub(super) async fn claim(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(Claim { code }): JsonBody<Claim>,
) -> Result<(StatusCode, Json<DeviceAnswer>), ApiError> {
    let services = &state.services;
    match services.claims.claim(&services.stores.of(&user.id).await?, &user.id, &code).await {
        Ok(device) => Ok((StatusCode::CREATED, Json(DeviceAnswer { device }))),
        Err(ClaimRefusal::TooManyAttempts) => Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "Too many claim attempts",
        )),
        Err(ClaimRefusal::UnknownCode) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::InvalidCode,
            "Unknown or expired pairing code",
        )),
        Err(ClaimRefusal::Storage(error)) => Err(error.into()),
    }
}
