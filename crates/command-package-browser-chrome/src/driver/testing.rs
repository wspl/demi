//! What only the Chrome tests reach.

/// The capture extension's ID, which a test finds the extension's targets by.
pub const CAPTURE_EXTENSION_ID: &str = crate::driver::launch::CAPTURE_EXTENSION_ID;

/// The installation the Chrome tests start (`builds-and-releases.md`
/// § Validation): the executable `DEMI_TEST_CHROME` names, and where the
/// pinned release starts with the Chrome runtime, on Linux, the runtime
/// unpacked in the directory `DEMI_TEST_CHROME_RUNTIME` names.
pub fn installation() -> crate::driver::installation::Installation {
    let executable = std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME");
    let release = demi_command_package_browser_protocol::release::BrowserRelease::pinned()
        .expect("the pinned release");
    let runtime = release
        .runtime_archives(demi_command_protocol::host_target())
        .map(|_| {
            let directory = std::env::var_os("DEMI_TEST_CHROME_RUNTIME")
                .expect("DEMI_TEST_CHROME_RUNTIME names the pinned Chrome runtime, unpacked");
            crate::driver::launch::Runtime::unpacked(std::path::Path::new(&directory))
        });
    crate::driver::installation::Installation {
        executable: executable.into(),
        runtime,
    }
}
