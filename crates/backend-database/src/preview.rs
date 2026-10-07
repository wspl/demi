//! The deployment's namespace at the preview domain (`storage.md` § Control
//! records; `preview.md` § The preview domain service): one row, the
//! namespace, its secret sealed by the vault, its expiry, and the origins it
//! was last registered with, so that a start knows whether to renew it or
//! replace them.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::PreviewNamespace;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::{decode, instant, to_json};
use super::control::ControlService;

const TABLE: &str = "preview_namespace";

/// The namespace as the control database keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRecord {
    pub namespace: PreviewNamespace,
    /// The namespace's secret, sealed for it by the vault.
    pub sealed_secret: Vec<u8>,
    pub expires_at: Timestamp,
    /// The origins the namespace admits, in the order they were registered.
    pub origins: Vec<String>,
}

impl ControlService {
    /// The deployment's namespace, if one was registered.
    pub async fn preview_namespace(&self) -> Result<Option<PreviewRecord>, StorageError> {
        self.call(|connection, _| {
            connection
                .query_row(
                    "SELECT namespace, secret, expires_at, origins FROM preview_namespace",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>("namespace")?,
                            row.get::<_, Vec<u8>>("secret")?,
                            instant(row, TABLE, "expires_at"),
                            row.get::<_, String>("origins")?,
                        ))
                    },
                )
                .optional()?
                .map(|(namespace, sealed_secret, expires_at, origins)| {
                    Ok(PreviewRecord {
                        namespace: decode(TABLE, "namespace", PreviewNamespace::try_from(namespace))?,
                        sealed_secret,
                        expires_at: expires_at?,
                        origins: decode(TABLE, "origins", serde_json::from_str(&origins))?,
                    })
                })
                .transpose()
        })
        .await
    }

    /// Keeps `record` as the deployment's namespace, in place of any other.
    pub async fn set_preview_namespace(&self, record: PreviewRecord) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO preview_namespace (id, namespace, secret, expires_at, origins)
                 VALUES (1, ?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET namespace = excluded.namespace,
                   secret = excluded.secret, expires_at = excluded.expires_at,
                   origins = excluded.origins",
                params![
                    record.namespace.as_str(),
                    record.sealed_secret,
                    record.expires_at.as_millisecond(),
                    to_json(&record.origins),
                ],
            )?;
            Ok(())
        })
        .await
    }
}
