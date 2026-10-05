//! The local object store (`storage.md` § The object store): the data
//! directory through `object_store`'s local file system, which syncs every
//! write and its directory before the write returns, so a local object is
//! durable once written, as an S3 object is. A file has no place for an
//! object's attributes, so they lie in a file of their own under
//! `.attributes/`, which no object's listing reaches: written before the
//! object, so an object never lacks the attributes it was written with.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::BoxStream;
use object_store::local::LocalFileSystem;
use object_store::path::{Path, PathPart};
use object_store::{
    Attribute, AttributeValue, Attributes, CopyOptions, GetOptions, GetResult, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt as _, PutMode, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, RenameOptions, Result,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where an object's attributes lie, beside the objects' own keys.
const ATTRIBUTES: &str = ".attributes";

/// The data directory as an object store.
#[derive(Debug)]
pub struct LocalObjects {
    files: Arc<LocalFileSystem>,
}

/// An object's attributes as their file holds them: the ones the backend
/// writes.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stored {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_encoding: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, String>,
}

impl Stored {
    /// What `attributes` store as; an attribute the backend never writes
    /// is refused.
    fn of(attributes: &Attributes) -> Result<Self> {
        let mut stored = Self::default();
        for (attribute, value) in attributes {
            let value = value.as_ref().to_owned();
            match attribute {
                Attribute::ContentType => stored.content_type = Some(value),
                Attribute::ContentEncoding => stored.content_encoding = Some(value),
                Attribute::Metadata(name) => {
                    stored.metadata.insert(name.as_ref().to_owned(), value);
                }
                other => {
                    return Err(object_store::Error::NotImplemented {
                        operation: format!("the attribute {other:?}"),
                        implementer: "the local object store".into(),
                    });
                }
            }
        }
        Ok(stored)
    }

    fn attributes(self) -> Attributes {
        let mut attributes = Attributes::new();
        if let Some(value) = self.content_type {
            attributes.insert(Attribute::ContentType, AttributeValue::from(value));
        }
        if let Some(value) = self.content_encoding {
            attributes.insert(Attribute::ContentEncoding, AttributeValue::from(value));
        }
        for (name, value) in self.metadata {
            attributes.insert(Attribute::Metadata(name.into()), AttributeValue::from(value));
        }
        attributes
    }
}

impl LocalObjects {
    /// The store rooted at `directory`, which exists.
    pub fn new(directory: &std::path::Path) -> Result<Self> {
        let files = LocalFileSystem::new_with_prefix(directory)?.with_fsync(true);
        Ok(Self {
            files: Arc::new(files),
        })
    }

    /// Where the attributes of the object at `location` lie.
    fn attributes_of(location: &Path) -> Path {
        std::iter::once(PathPart::from(ATTRIBUTES))
            .chain(location.parts())
            .collect()
    }

    /// Whether `location` is where some object's attributes lie, which no
    /// listing shows.
    fn holds_attributes(location: &Path) -> bool {
        location
            .parts()
            .next()
            .is_some_and(|first| first.as_ref() == ATTRIBUTES)
    }

    /// The attributes of the object at `location`, none when it has none.
    async fn read_attributes(&self, location: &Path) -> Result<Attributes> {
        let path = Self::attributes_of(location);
        let bytes = match self.files.get(&path).await {
            Ok(found) => found.bytes().await?,
            Err(object_store::Error::NotFound { .. }) => return Ok(Attributes::new()),
            Err(error) => return Err(error),
        };
        let stored: Stored =
            serde_json::from_slice(&bytes).map_err(|error| object_store::Error::Generic {
                store: "the local object store",
                source: format!("the attributes of {location} are corrupt: {error}").into(),
            })?;
        Ok(stored.attributes())
    }
}

impl fmt::Display for LocalObjects {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LocalObjects({})", self.files)
    }
}

#[async_trait]
impl ObjectStore for LocalObjects {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        let PutOptions {
            mode,
            tags,
            attributes,
            extensions,
        } = opts;
        if !attributes.is_empty() {
            // The attributes go first. When their file is there already, as
            // a put that stopped before its object left it, the object is
            // created only with the same attributes; with others, the
            // object counts as one that exists, which the caller reads.
            let bytes = serde_json::to_vec(&Stored::of(&attributes)?).map_err(|error| {
                object_store::Error::Generic {
                    store: "the local object store",
                    source: error.into(),
                }
            })?;
            let create = PutOptions {
                mode: mode.clone(),
                ..PutOptions::default()
            };
            let path = Self::attributes_of(location);
            match self
                .files
                .put_opts(&path, PutPayload::from(Bytes::from(bytes.clone())), create)
                .await
            {
                Ok(_) => {}
                Err(object_store::Error::AlreadyExists { .. }) => {
                    let existing = self.files.get(&path).await?.bytes().await?;
                    if existing != bytes {
                        return Err(object_store::Error::AlreadyExists {
                            path: location.to_string(),
                            source: "the object exists with other attributes".into(),
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
        let opts = PutOptions {
            mode,
            tags,
            attributes: Attributes::new(),
            extensions,
        };
        self.files.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        // The backend writes no object in parts; one with attributes the
        // local file system refuses.
        self.files.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        let mut found = self.files.get_opts(location, options).await?;
        found.attributes = self.read_attributes(location).await?;
        Ok(found)
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        // An object's attributes go after it: an object never lacks its
        // attributes, while attributes left without their object are
        // overwritten by the next put of that key.
        use futures_util::StreamExt as _;
        let files = self.files.clone();
        self.files
            .delete_stream(locations)
            .then(move |deleted| {
                let files = files.clone();
                async move {
                    let location = deleted?;
                    match files.delete(&Self::attributes_of(&location)).await {
                        Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(location),
                        Err(error) => Err(error),
                    }
                }
            })
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        use futures_util::StreamExt as _;
        self.files
            .list(prefix)
            .filter(|listed| {
                let shown = !listed
                    .as_ref()
                    .is_ok_and(|meta| Self::holds_attributes(&meta.location));
                std::future::ready(shown)
            })
            .boxed()
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        let mut listed = self.files.list_with_delimiter(prefix).await?;
        listed
            .common_prefixes
            .retain(|common| !Self::holds_attributes(common));
        listed
            .objects
            .retain(|meta| !Self::holds_attributes(&meta.location));
        Ok(listed)
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
        self.copy_attributes(from, to).await?;
        self.files.copy_opts(from, to, options).await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
        self.copy_attributes(from, to).await?;
        self.files.rename_opts(from, to, options).await
    }
}

impl LocalObjects {
    /// Gives the object at `to` the attributes of the one at `from`, before
    /// a copy or a rename moves its bytes.
    async fn copy_attributes(&self, from: &Path, to: &Path) -> Result<()> {
        let attributes = self.read_attributes(from).await?;
        if attributes.is_empty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&Stored::of(&attributes)?).map_err(|error| {
            object_store::Error::Generic {
                store: "the local object store",
                source: error.into(),
            }
        })?;
        let replace = PutOptions {
            mode: PutMode::Overwrite,
            ..PutOptions::default()
        };
        self.files
            .put_opts(&Self::attributes_of(to), Bytes::from(bytes).into(), replace)
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, LocalObjects) {
        let directory = tempfile::tempdir().unwrap();
        let objects = LocalObjects::new(directory.path()).unwrap();
        (directory, objects)
    }

    fn attributes(sha256: &str) -> Attributes {
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentEncoding, "zstd".into());
        attributes.insert(Attribute::Metadata("sha256".into()), sha256.to_owned().into());
        attributes
    }

    fn create(attributes: Attributes) -> PutOptions {
        PutOptions {
            mode: PutMode::Create,
            attributes,
            ..PutOptions::default()
        }
    }

    #[tokio::test]
    async fn an_object_keeps_its_attributes_and_a_second_create_finds_it() {
        let (directory, objects) = store();
        let key = Path::from("native/blobs/abc");
        objects
            .put_opts(&key, PutPayload::from_static(b"encoded"), create(attributes("abc")))
            .await
            .unwrap();
        let head = GetOptions {
            head: true,
            ..GetOptions::default()
        };
        let found = objects.get_opts(&key, head).await.unwrap();
        assert_eq!(found.attributes, attributes("abc"));
        assert_eq!(found.meta.size, 7);
        // A listing of the objects never shows their attributes.
        let listed: Vec<_> = futures_util::TryStreamExt::try_collect::<Vec<_>>(objects.list(None))
            .await
            .unwrap()
            .into_iter()
            .map(|meta| meta.location.to_string())
            .collect();
        assert_eq!(listed, ["native/blobs/abc"]);
        assert!(directory.path().join(".attributes/native/blobs/abc").is_file());
        // Creating it again with its attributes finds it; with others too,
        // and the caller reads which it found.
        for again in [attributes("abc"), attributes("other")] {
            let refused = objects
                .put_opts(&key, PutPayload::from_static(b"encoded"), create(again))
                .await;
            assert!(
                matches!(refused, Err(object_store::Error::AlreadyExists { .. })),
                "{refused:?}"
            );
        }
        assert_eq!(objects.get_opts(&key, GetOptions::default()).await.unwrap().attributes, attributes("abc"));
    }

    #[tokio::test]
    async fn a_put_that_stopped_after_its_attributes_completes_and_a_deletion_takes_both() {
        let (directory, objects) = store();
        let key = Path::from("native/blobs/abc");
        // A put that stopped between its attributes and its object.
        let stored = serde_json::to_vec(&Stored::of(&attributes("abc")).unwrap()).unwrap();
        let left = directory.path().join(".attributes/native/blobs");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::write(left.join("abc"), stored).unwrap();
        objects
            .put_opts(&key, PutPayload::from_static(b"encoded"), create(attributes("abc")))
            .await
            .unwrap();
        objects.delete(&key).await.unwrap();
        assert!(!left.join("abc").exists());
        // An object without attributes has none.
        let plain = Path::from("blobs/ana/def");
        objects.put(&plain, PutPayload::from_static(b"picture")).await.unwrap();
        let found = objects.get_opts(&plain, GetOptions::default()).await.unwrap();
        assert!(found.attributes.is_empty());
    }
}
