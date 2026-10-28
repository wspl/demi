//! A project on the user's Cloud: a new project directory there, named
//! by the project's name.
//!
//! It is made through the Cloud's machine access
//! (`crates/backend-host-access/src/lib.rs`), which wakes a stopped Cloud.

use demi_backend_database::projects::{ProjectCreation, ProjectRecord, new_project_id};
use demi_web_api_protocol::ids::WorkspaceId;
use demi_host_interface::{Host as _, HostErrorKind, HostFs, MkdirOptions};

use crate::shard::Shard;
use demi_backend_host_access::access::{HostAccessError, Refusal};

/// What a new Cloud project came to.
#[derive(Debug)]
pub enum CloudProject {
    /// Made, or brought back: the user had deleted the directory's project.
    Made(ProjectRecord),
    /// A folder of the name is in `~/projects` already.
    NameTaken,
    /// The name cannot name a folder.
    NameInvalid,
    /// The workspace is not the user's.
    WorkspaceNotFound,
}

impl Shard {
    /// Makes `~/projects/<name>` on the user's Cloud, then the project over
    /// it in the user's `workspace`. Every project of the user is a directory
    /// on the same Cloud.
    ///
    /// A project the user deleted of that directory comes back, its folder
    /// with what it holds; any other folder of the name, made by a project
    /// or not, refuses the name, as a file manager refuses a second folder
    /// of one name.
    pub async fn create_cloud_project(
        &self,
        workspace: WorkspaceId,
        name: String,
    ) -> Result<CloudProject, HostAccessError> {
        if name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
            return Ok(CloudProject::NameInvalid);
        }
        let access = self.cloud_shard().machine_access().await?;
        let paths = access.host.identity().paths;
        let projects = paths.join(&access.home, "projects");
        let path = paths.join(&projects, &name);
        let control = &self.services().control;
        let kept = control
            .project_at(workspace.clone(), access.device.id.clone(), path.clone())
            .await?;
        match kept {
            Some(kept) if !kept.deleted => return Ok(CloudProject::NameTaken),
            Some(_) => {}
            None => {
                HostFs::mkdir(&access.host, &projects, MkdirOptions { recursive: true }).await?;
                if let Err(error) = HostFs::mkdir(&access.host, &path, MkdirOptions { recursive: false }).await {
                    return match error.kind {
                        HostErrorKind::Failed { code: Some(code) } if code == "EEXIST" => Ok(CloudProject::NameTaken),
                        _ => Err(error.into()),
                    };
                }
            }
        }
        match control
            .create_project(new_project_id(), self.user().clone(), workspace, access.device.id.clone(), path)
            .await?
        {
            ProjectCreation::Created(project) => Ok(CloudProject::Made(project)),
            ProjectCreation::Exists(_) => Ok(CloudProject::NameTaken),
            ProjectCreation::WorkspaceNotFound => Ok(CloudProject::WorkspaceNotFound),
            ProjectCreation::DeviceNotFound | ProjectCreation::Home => Err(Refusal::DeviceGone.into()),
        }
    }
}
