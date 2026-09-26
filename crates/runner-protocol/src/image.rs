//! What a Cloud image holds for the runner (`images.md` § Root filesystem
//! contents): the image build writes it, the manager's image check expects
//! it, and the runner reads it (`native-runtime.md` § Preinstalled
//! executables).

/// Where an image embeds each command package's executable: alone in a
/// directory named by its SHA-256, under the name its release gives it, such
/// as `/opt/demi/artifacts/<sha256>/demi-commands`.
pub const ARTIFACTS_PATH: &str = "/opt/demi/artifacts";
