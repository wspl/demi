//! The object store that holds the blobs (`storage.md` § The object store):
//! the data directory of a single-backend deployment, or the S3 bucket
//! `DEMI_OBJECT_STORE_CONFIG` names, reached through `object_store` either
//! way, so a blob is the object `blobs/<user>/<sha256>`. S3's credentials
//! come from the standard AWS environment variables, a web identity token,
//! or container or instance metadata; no profile file is read.

use std::path::Path;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::aws::{AmazonS3Builder, Checksum};
use object_store::local::LocalFileSystem;
use serde::Deserialize;
use url::Url;

use crate::ObjectError;

/// Where `DEMI_OBJECT_STORE_CONFIG` puts the object store: an S3 bucket.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    /// An S3-compatible service instead of AWS, over HTTPS.
    #[serde(default)]
    pub endpoint: Option<Url>,
    /// Names the bucket in the request path instead of the host name.
    #[serde(default)]
    pub force_path_style: bool,
}

/// Why the S3 configuration cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum S3ConfigError {
    #[error("{0}")]
    Read(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

impl S3Config {
    /// The configuration in the JSON file at `path`, checked.
    pub async fn read(path: &Path) -> Result<Self, S3ConfigError> {
        let bytes = tokio::fs::read(path).await?;
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| S3ConfigError::Invalid(error.to_string()))?;
        config.check()?;
        Ok(config)
    }

    /// Checks what the JSON's shape cannot: a bucket and a region, and an
    /// HTTPS endpoint.
    pub fn check(&self) -> Result<(), S3ConfigError> {
        if self.bucket.is_empty() || self.region.is_empty() {
            return Err(S3ConfigError::Invalid("bucket and region must not be empty".into()));
        }
        if self.endpoint.as_ref().is_some_and(|endpoint| endpoint.scheme() != "https") {
            return Err(S3ConfigError::Invalid("the endpoint must be an HTTPS URL".into()));
        }
        Ok(())
    }

    /// The bucket's client, which checksums every upload with SHA-256 and
    /// creates an object only if its key is free when asked to.
    pub fn builder(&self) -> AmazonS3Builder {
        let builder = AmazonS3Builder::from_env()
            .with_bucket_name(&self.bucket)
            .with_region(&self.region)
            .with_checksum_algorithm(Checksum::SHA256);
        match &self.endpoint {
            Some(endpoint) if self.force_path_style => builder
                .with_endpoint(endpoint.as_str().trim_end_matches('/'))
                .with_virtual_hosted_style_request(false),
            Some(endpoint) => {
                // A virtual-hosted endpoint names the bucket in its host.
                let mut hosted = endpoint.clone();
                let host = format!("{}.{}", self.bucket, endpoint.host_str().unwrap_or_default());
                // A URL that has a host takes another.
                let _ = hosted.set_host(Some(&host));
                builder
                    .with_endpoint(hosted.as_str().trim_end_matches('/'))
                    .with_virtual_hosted_style_request(true)
            }
            None => builder.with_virtual_hosted_style_request(!self.force_path_style),
        }
    }
}

/// The object store: the S3 bucket `s3` names, or the data directory.
pub async fn open(data_dir: &Path, s3: Option<&S3Config>) -> Result<Arc<dyn ObjectStore>, ObjectError> {
    if let Some(config) = s3 {
        return Ok(Arc::new(config.builder().build()?));
    }
    let root = data_dir.to_owned();
    // Resolving the directory is file system work.
    let local = tokio::task::spawn_blocking(move || LocalFileSystem::new_with_prefix(root)).await??;
    Ok(Arc::new(local))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use demi_web_api::ids::UserId;

    use crate::fake_s3::FakeS3;
    use object_store::ObjectStore;

    use super::*;
    use crate::blobs::BlobStores;

    #[tokio::test]
    async fn an_s3_bucket_holds_each_blob_once_under_its_users_namespace() {
        let fake = FakeS3::start().await;
        let objects: Arc<dyn ObjectStore> = Arc::new(fake.client());
        let blobs = BlobStores::new(objects, Arc::new(demi_core::SystemClock)).for_user(&UserId::try_from("ana").unwrap());
        let first = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        // The same bytes again are the same blob, created once.
        let again = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        assert_eq!(first, again);
        assert_eq!(blobs.get(&first).await.unwrap(), Some(Bytes::from_static(b"picture")));
        assert_eq!(fake.written(), [format!("blobs/ana/{first}")]);
    }

    #[tokio::test]
    async fn the_s3_configuration_names_a_bucket_and_region_over_https() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("s3.json");
        let write = |text: &str| std::fs::write(&path, text).unwrap();
        write(r#"{"bucket":"demi","region":"eu-west-1","endpoint":"https://objects.example.com","forcePathStyle":true}"#);
        let config = S3Config::read(&path).await.unwrap();
        assert!(config.force_path_style);
        for refused in [
            r#"{"bucket":"demi","region":"eu-west-1","endpoint":"http://objects.example.com"}"#,
            r#"{"bucket":"","region":"eu-west-1"}"#,
            r#"{"bucket":"demi","region":"eu-west-1","profile":"default"}"#,
            r#"{"region":"eu-west-1"}"#,
        ] {
            write(refused);
            assert!(S3Config::read(&path).await.is_err(), "{refused}");
        }
    }
}
