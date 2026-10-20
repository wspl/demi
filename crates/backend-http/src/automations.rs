//! The caller's automations, which only their owner reads and changes.
//!
//! - Each route acts in the caller's shard.
//! - A change reaches the caller's pages as the `automations` part.
//! - An automation's URL, `/hooks/<id>`, is authenticated by no session: its id
//!   and its secret are its only keys.

use std::time::Instant;

use axum::extract::{Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use demi_backend_user_shard::automations::{AutomationError, CALLS_PER_MINUTE, HookAnswer, HookCall, check_unknown};
use demi_shared_types::AutomationId;
use demi_web_api_protocol::automations::{
    AutomationAnswer, AutomationList, AutomationPatch, AutomationSecret, HolidayCountries, RunNow,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ConversationId;

use super::AppState;
use super::body::{JsonBody, read};
use super::error::{ApiError, status_of};
use super::gate::AuthUser;
use super::page_client::client_address;

/// The one answer an unknown address and a wrong secret share.
fn unauthorized() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthenticated,
        "The call carries no valid secret for this address",
    )
}

fn too_many() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::RateLimited,
        format!("This address takes {CALLS_PER_MINUTE} calls a minute"),
    )
}

/// `POST /hooks/:id`: a call of an automation's URL. An unknown address
/// costs the same checks as a wrong secret, and gets the same answer.
pub(super) async fn hook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let limits = &state.services.hook_limits;
    let now = Instant::now();
    let (parts, body) = request.into_parts();
    let client = client_address(&parts);
    let id = AutomationId::try_from(id).ok();
    let owner = match &id {
        Some(id) => state.services.control.automation_owner(id.clone()).await?,
        None => None,
    };
    let admitted = match (&id, &owner) {
        (Some(id), Some(_)) => limits.admit_known(id, now),
        _ => limits.admit_unknown(client, now),
    };
    if !admitted {
        return Err(too_many());
    }
    let method = parts.method.to_string();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .collect();
    let post = parts.method == Method::POST;
    let body = read(Request::from_parts(parts, body), &state).await?;
    let call = HookCall { method, headers, body };
    let (Some(id), Some(owner), true) = (id, owner, post) else {
        check_unknown(state.services.vault.key(), &call, state.services.control.now());
        return Err(unauthorized());
    };
    let answer = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.hook_call(&id, call).await })
        .await??;
    match answer {
        HookAnswer::Taken => Ok(StatusCode::OK.into_response()),
        HookAnswer::Refused => Err(unauthorized()),
    }
}

/// `GET /automations`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<AutomationList>, ApiError> {
    let automations = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.automation_list().await })
        .await??;
    Ok(Json(AutomationList { automations }))
}

/// `POST /automations`: answers 201 with the new automation.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(patch): JsonBody<AutomationPatch>,
) -> Result<(StatusCode, Json<AutomationAnswer>), ApiError> {
    let (automation, _) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_automation(patch).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(AutomationAnswer { automation })))
}

/// `PATCH /automations/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<AutomationPatch>,
) -> Result<Json<AutomationAnswer>, ApiError> {
    let id = automation_id(id)?;
    let (automation, _) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.change_automation(&id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(AutomationAnswer { automation }))
}

/// `DELETE /automations/:id`.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = automation_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.delete_automation(&id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /automations/:id/run`: Run Now, 202.
pub(super) async fn run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<RunNow>,
) -> Result<StatusCode, ApiError> {
    let id = automation_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.run_automation_now(&id, body.task_id).await.map(drop) })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::ACCEPTED)
}

/// `GET /automations/holidays`: the countries and regions a task's working
/// days can follow, the same for every user.
pub(super) async fn holidays(AuthUser(_): AuthUser) -> Json<HolidayCountries> {
    Json(HolidayCountries {
        countries: demi_backend_user_shard::automations::holiday_countries(),
    })
}

/// `GET /automations/:id/secret`: Show.
pub(super) async fn secret(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<AutomationSecret>, ApiError> {
    let id = automation_id(id)?;
    let secret = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.automation_secret(&id).await })
        .await?
        .map_err(refused)?;
    Ok(Json(AutomationSecret { secret }))
}

/// `POST /automations/:id/secret`: Regenerate Secret.
pub(super) async fn regenerate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<AutomationSecret>, ApiError> {
    let id = automation_id(id)?;
    let secret = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.regenerate_automation_secret(&id).await })
        .await?
        .map_err(refused)?;
    Ok(Json(AutomationSecret { secret }))
}

/// `POST /conversations/:id/move-to-conversations`: a run's conversation
/// joins the sidebar, 204.
pub(super) async fn move_to_conversations(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = ConversationId::try_from(id).map_err(|_| refused(AutomationError::NotFiled))?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.move_to_conversations(&id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn automation_id(id: String) -> Result<AutomationId, ApiError> {
    AutomationId::try_from(id).map_err(|_| refused(AutomationError::NotFound))
}

fn refused(error: AutomationError) -> ApiError {
    let message = error.to_string();
    let (status, code) = match error {
        AutomationError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::AutomationNotFound),
        AutomationError::Cron(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidCron),
        AutomationError::TimeZone(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidTimeZone),
        AutomationError::ConversationUnavailable => {
            (StatusCode::CONFLICT, ErrorCode::AutomationConversationUnavailable)
        }
        AutomationError::LimitNotTaken => (StatusCode::BAD_REQUEST, ErrorCode::LimitNotTaken),
        AutomationError::NotUrl => (StatusCode::CONFLICT, ErrorCode::NotUrlTrigger),
        AutomationError::NotFiled => (StatusCode::CONFLICT, ErrorCode::NotFiled),
        AutomationError::Holidays(_) => (StatusCode::BAD_REQUEST, ErrorCode::UnknownHolidays),
        AutomationError::NoTask(_) | AutomationError::TaskNeeded | AutomationError::NoTasks => {
            (StatusCode::BAD_REQUEST, ErrorCode::TaskNotFound)
        }
        AutomationError::NoMessage => (StatusCode::CONFLICT, ErrorCode::TaskWithoutMessage),
        AutomationError::Change(refusal) => {
            if let demi_backend_host_access::transition::ChangeRefusal::Storage(error) = refusal {
                return error.into();
            }
            let (code, status) = refusal.code();
            (status_of(status), code)
        }
        AutomationError::Storage(error) => return error.into(),
    };
    ApiError::new(status, code, message)
}
