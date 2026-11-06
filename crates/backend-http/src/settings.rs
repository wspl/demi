//! The instance's settings, which only the master changes: the instance
//! mode, and the Cloud's isolation and network.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_cloud::client::MachinesError;
use demi_backend_cloud::isolation;
use demi_backend_cloud::network::{self, AllowedError};
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::settings::{
    CloudIsolation, CloudNetwork, SetCloudIsolation, SetCloudNetwork, SetInstanceMode, Settings,
};

use demi_core_api::body::JsonBody;
use demi_core_api::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

pub(super) async fn settings(State(services): State<Arc<Services>>) -> Json<Settings> {
    Json(Settings {
        mode: services.vault.mode(),
    })
}

/// Changes the instance mode, which only the master does: the master's are
/// the entries a shared instance's users infer with.
pub(super) async fn set_instance_mode(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<SetInstanceMode>,
) -> Result<Json<Settings>, ApiError> {
    if user.role != Role::Master {
        return Err(ApiError::forbidden("Only the master changes the instance mode"));
    }
    services.vault.set_mode(request.mode).await?;
    Ok(Json(Settings { mode: request.mode }))
}

/// Settings › Cloud's isolation, which only the master reads.
pub(super) async fn cloud_isolation(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<CloudIsolation>, ApiError> {
    if user.role != Role::Master {
        return Err(ApiError::forbidden("Only the master sees how Clouds are isolated"));
    }
    Ok(Json(isolation::setting(&services.control, &services.cloud).await?))
}

/// Changes how Clouds are isolated, which only the master does: one the
/// machine manager says it cannot run is refused. A manager that cannot be
/// reached gets the stored choice when the backend connects to it again;
/// one that refuses it leaves the choice as it was.
pub(super) async fn set_cloud_isolation(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<SetCloudIsolation>,
) -> Result<Json<CloudIsolation>, ApiError> {
    if user.role != Role::Master {
        return Err(ApiError::forbidden("Only the master changes how Clouds are isolated"));
    }
    if let Some(reason) = isolation::unavailable(&services.cloud, request.isolation) {
        return Err(ApiError::invalid_body(format!("This server can't run it: {reason}")));
    }
    match isolation::apply(&services.cloud, request.isolation).await {
        Ok(()) => {
            services.control.set_isolation(request.isolation).await?;
            services.isolation_changed();
        }
        Err(error @ MachinesError::Unavailable { .. }) => {
            services.control.set_isolation(request.isolation).await?;
            services.isolation_changed();
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::CloudUnavailable,
                format!("Saved. Clouds get it when the machine manager is back: {error}"),
            ));
        }
        Err(error) => {
            return Err(ApiError::internal_message(format!(
                "The machine manager did not take the isolation: {error}"
            )));
        }
    }
    Ok(Json(isolation::setting(&services.control, &services.cloud).await?))
}

/// Settings › Cloud's network policy, which only the master reads.
pub(super) async fn cloud_network(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<CloudNetwork>, ApiError> {
    if user.role != Role::Master {
        return Err(ApiError::forbidden("Only the master sees the network policy"));
    }
    Ok(Json(network::setting(&services.control).await?))
}

/// Changes the network policy, which only the master does. An entry of
/// Custom's list that is not one refuses the change, naming its place in the
/// list (`index`). A manager that cannot be reached gets the stored policy
/// when the backend connects to it again; one that refuses it leaves the
/// policy as it was.
pub(super) async fn set_cloud_network(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<SetCloudNetwork>,
) -> Result<Json<CloudNetwork>, ApiError> {
    if user.role != Role::Master {
        return Err(ApiError::forbidden("Only the master changes the network policy"));
    }
    let rules = network::network_rules(request.policy, &request.custom).map_err(|invalid| {
        let problem = match invalid.error {
            AllowedError::Entry(_) => "entry",
            AllowedError::Port(_) => "port",
            AllowedError::NeverReachable(_) => "neverReachable",
        };
        ApiError::invalid_body(invalid.to_string())
            .with_param("index", invalid.index)
            .with_param("problem", problem)
    })?;
    match network::apply(&services.cloud, rules).await {
        Ok(()) => {
            services.control.set_network_policy(request.policy, request.custom).await?;
            Ok(Json(network::setting(&services.control).await?))
        }
        Err(error @ MachinesError::Unavailable { .. }) => {
            services.control.set_network_policy(request.policy, request.custom).await?;
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::CloudUnavailable,
                format!("Saved. Clouds get it when the machine manager is back: {error}"),
            ))
        }
        Err(error) => Err(ApiError::internal_message(format!(
            "The machine manager did not apply the network policy: {error}"
        ))),
    }
}
