//! The artifacts a service's program asks for (`native-runtime.md` § The
//! artifacts stream): each install names the invocation it serves, which
//! the runner started for one job or one stream and registered here with
//! that work's resolver, so the backend locates the artifact for that work.
//! An install held by a running service is not removed with its line's
//! older artifacts while the service lives.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use demi_command_protocol::{ArtifactAsk, ArtifactReply, PackageArtifact};
use tokio_util::sync::CancellationToken;

use crate::ArtifactResolver;
use crate::cache::{ArtifactCache, Hold, Wanted};

/// The invocations running now, by id: the package each runs in, the
/// resolver of the work it serves, and a token its end cancels. Cloning
/// shares the table.
#[derive(Clone, Default)]
pub struct Invocations(Arc<Mutex<HashMap<String, Registered>>>);

#[derive(Clone)]
struct Registered {
    package: String,
    resolver: Arc<dyn ArtifactResolver>,
    ended: CancellationToken,
}

/// One registered invocation, which leaves the table when dropped and so
/// ends the installs it waits for.
pub struct Invoking {
    invocations: Invocations,
    id: String,
    ended: CancellationToken,
}

impl Invocations {
    /// Registers `id`, an invocation of `package` serving the work
    /// `resolver` locates artifacts for, until the returned guard drops.
    pub fn register(
        &self,
        id: &str,
        package: &str,
        resolver: Arc<dyn ArtifactResolver>,
    ) -> Invoking {
        let ended = CancellationToken::new();
        let registered = Registered {
            package: package.to_owned(),
            resolver,
            ended: ended.clone(),
        };
        self.table().insert(id.to_owned(), registered);
        Invoking {
            invocations: self.clone(),
            id: id.to_owned(),
            ended,
        }
    }

    fn get(&self, id: &str, package: &str) -> Option<Registered> {
        self.table()
            .get(id)
            .filter(|registered| registered.package == package)
            .cloned()
    }

    fn table(&self) -> MutexGuard<'_, HashMap<String, Registered>> {
        self.0.lock().expect("the invocations are intact")
    }
}

impl Drop for Invoking {
    fn drop(&mut self) {
        self.ended.cancel();
        let mut table = self.invocations.table();
        // A later registration of the same id is not this one's to remove.
        if table
            .get(&self.id)
            .is_some_and(|registered| registered.ended.is_cancelled())
        {
            table.remove(&self.id);
        }
    }
}

/// Answers one service's artifacts stream.
pub(crate) struct ServiceArtifacts {
    pub(crate) cache: Arc<ArtifactCache>,
    pub(crate) package: String,
    pub(crate) invocations: Invocations,
    /// What the service was given, held while it lives.
    pub(crate) held: Mutex<Vec<Hold>>,
}

impl ServiceArtifacts {
    pub(crate) async fn answer(&self, ask: ArtifactAsk) -> Result<ArtifactReply, String> {
        match ask {
            ArtifactAsk::Install(install) => {
                let invocation = self
                    .invocations
                    .get(&install.invocation, &self.package)
                    .ok_or_else(|| {
                        format!(
                            "{} is no running invocation of {}",
                            install.invocation, self.package
                        )
                    })?;
                let wanted = Wanted {
                    package: &self.package,
                    name: &install.name,
                    version: &install.version,
                    artifact: PackageArtifact {
                        sha256: install.sha256.clone(),
                        size: install.size,
                    },
                    form: &install.form,
                };
                // Held from the start, so an install of another version of
                // the line meanwhile leaves it.
                let hold = self.cache.holds().hold(&install.sha256);
                let path = self
                    .cache
                    .install(&wanted, invocation.resolver.as_ref(), &invocation.ended)
                    .await
                    .map_err(|error| error.to_string())?;
                self.held.lock().expect("the holds are intact").push(hold);
                Ok(ArtifactReply::Path(path.to_string_lossy().into_owned()))
            }
            ArtifactAsk::Installed(question) => self
                .cache
                .installed(&self.package, &question.name)
                .await
                .map(ArtifactReply::Installed)
                .map_err(|error| error.to_string()),
        }
    }
}
