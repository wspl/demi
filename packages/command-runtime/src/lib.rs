//! Runner-owned native artifact cache and resident service lifetimes.

mod cache;
mod process;

pub use cache::{ArtifactCache, ArtifactResolver, ArtifactSource, RuntimeError};
pub use process::ResidentService;
