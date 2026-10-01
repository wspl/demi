//! What only the Chrome tests reach.

/// The capture extension's ID, which a test finds the extension's targets by.
pub const CAPTURE_EXTENSION_ID: &str = crate::driver::launch::CAPTURE_EXTENSION_ID;

/// Installs the pinned Chrome for Testing whose unpacked executable
/// `executable` is, such as the one `DEMI_TEST_CHROME` names, into the
/// browser directory `root`, as the service installs a release, so that a
/// service whose browser directory `root` is finds it: no test downloads
/// Chrome. A Host user's `root` is `.demi/browsers` in their home.
#[cfg(feature = "testing")]
pub async fn install_pinned(
    root: &std::path::Path,
    executable: &std::path::Path,
) -> crate::driver::operation::Result<std::path::PathBuf> {
    let archive = crate::driver::installation::pinned_archive()?;
    demi_shared_artifacts::testing::install_unpacked(
        root,
        &archive,
        executable,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .map_err(|error| crate::driver::operation::BrowserError::Installation(error.to_string()))
}
