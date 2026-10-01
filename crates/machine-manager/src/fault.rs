//! Crash points for acceptance (`managed-hosts.md` § Verification): a
//! manager built with the `fault-injection` feature aborts at the point
//! `DEMI_MACHINES_FAULT` names, and the next start must recover with nothing
//! left behind. Other builds ignore every point.

/// Aborts the process here when this point is the injected fault.
pub fn point(name: &str) {
    #[cfg(feature = "fault-injection")]
    if std::env::var_os("DEMI_MACHINES_FAULT").is_some_and(|fault| fault == name) {
        eprintln!("demi-machines: injected fault at {name}");
        std::process::abort();
    }
    #[cfg(not(feature = "fault-injection"))]
    let _ = name;
}
