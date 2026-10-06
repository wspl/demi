//! The deployment's one object store (`storage.md` § The object store): the
//! data directory, or the S3 bucket the `DEMI_S3_*` settings name, reached
//! through `object_store` either way. It holds the users' blobs and the
//! published command packages under the same keys whichever it is. S3's
//! credentials come from the standard AWS environment variables, a web
//! identity token, or container or instance metadata; no profile file is
//! read.

use std::path::Path;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::aws::{AmazonS3Builder, Checksum};
use object_store::signer::Signer;
use url::Url;

use crate::ObjectError;
use crate::local::LocalObjects;

/// Where the deployment's object store lives (`DEMI_STORAGE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage {
    /// The data directory.
    Local,
    /// An S3 bucket.
    S3(S3Config),
}

/// The bucket of an S3 store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    /// An S3-compatible service instead of AWS, over HTTPS.
    pub endpoint: Option<Url>,
    /// Names the bucket in the request path instead of the host name.
    pub force_path_style: bool,
}

/// Why the S3 settings cannot be used.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct S3ConfigError(String);

impl S3Config {
    /// Checks what the settings' types cannot: a bucket and a region, and an
    /// HTTPS endpoint.
    pub fn check(&self) -> Result<(), S3ConfigError> {
        if self.bucket.is_empty() || self.region.is_empty() {
            return Err(S3ConfigError(
                "DEMI_S3_BUCKET and DEMI_S3_REGION must not be empty".into(),
            ));
        }
        if self
            .endpoint
            .as_ref()
            .is_some_and(|endpoint| endpoint.scheme() != "https")
        {
            return Err(S3ConfigError(
                "DEMI_S3_ENDPOINT must be an HTTPS URL".into(),
            ));
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
                let host = format!(
                    "{}.{}",
                    self.bucket,
                    endpoint.host_str().unwrap_or_default()
                );
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

/// The opened object store, and for S3 the signer of the download URLs a
/// runner fetches a command artifact from; a local store's downloads go
/// through the backend instead.
#[derive(Clone)]
pub struct Objects {
    pub store: Arc<dyn ObjectStore>,
    pub signer: Option<Arc<dyn Signer>>,
}

/// Opens the object store `storage` names: the S3 bucket, or the data
/// directory, which it creates when it does not exist.
pub async fn open(data_dir: &Path, storage: &Storage) -> Result<Objects, ObjectError> {
    match storage {
        Storage::S3(config) => {
            let bucket = Arc::new(config.builder().build()?);
            Ok(Objects {
                store: bucket.clone(),
                signer: Some(bucket),
            })
        }
        Storage::Local => {
            let root = data_dir.to_owned();
            // Making and resolving the directory is file system work.
            let local = tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&root).map_err(|error| object_store::Error::Generic {
                    store: "the local object store",
                    source: error.into(),
                })?;
                LocalObjects::new(&root)
            })
            .await??;
            Ok(Objects {
                store: Arc::new(local),
                signer: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use demi_web_api_protocol::ids::UserId;

    use crate::fake_s3::FakeS3;
    use object_store::ObjectStore;

    use super::*;
    use crate::blobs::BlobStores;

    #[tokio::test]
    async fn an_s3_bucket_holds_each_blob_once_under_its_users_namespace() {
        let fake = FakeS3::start().await;
        let objects: Arc<dyn ObjectStore> = Arc::new(fake.client());
        let blobs = BlobStores::new(objects)
            .for_user(&UserId::try_from("ana").unwrap());
        let first = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        // The same bytes again are the same blob, created once.
        let again = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        assert_eq!(first, again);
        assert_eq!(
            blobs.get(&first).await.unwrap(),
            Some(Bytes::from_static(b"picture"))
        );
        assert_eq!(fake.written(), [format!("blobs/ana/{first}")]);
    }

    #[test]
    fn the_s3_settings_name_a_bucket_and_region_over_https() {
        let config = |bucket: &str, endpoint: Option<&str>| S3Config {
            bucket: bucket.to_owned(),
            region: "eu-west-1".to_owned(),
            endpoint: endpoint.map(|endpoint| endpoint.parse().unwrap()),
            force_path_style: true,
        };
        config("demi", Some("https://objects.example.com"))
            .check()
            .unwrap();
        assert!(config("demi", Some("http://objects.example.com")).check().is_err());
        assert!(config("", None).check().is_err());
    }
}
