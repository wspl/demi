//! The usage ledger's totals of the caller's core
//! (`crates/core-store/src/usage.rs`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api_protocol::usage::UsageTotals;

use crate::error::ApiError;
use crate::caller::Principal;
use demi_core::services::CoreServices;

pub(crate) async fn totals(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
) -> Result<Json<UsageTotals>, ApiError> {
    let totals = services.vault.stores().store_of(&user).await?.usage_totals().await?;
    Ok(Json(UsageTotals { totals }))
}
