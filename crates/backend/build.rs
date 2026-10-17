// The Git branch the backend is built from, which picks the media catalog
// it downloads (`package-versioning.md` § Rust executables): the release
// workflow's `GITHUB_REF_NAME`, else the checkout's branch, else none, for a
// checkout on no branch or outside Git.
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-env-changed=GITHUB_REF_NAME");
    // A checkout that moves to another branch rewrites HEAD.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo::rerun-if-changed={head}");
    }
    let branch = std::env::var("GITHUB_REF_NAME")
        .ok()
        .filter(|name| !name.is_empty())
        .or_else(|| git(&["symbolic-ref", "--quiet", "--short", "HEAD"]));
    if let Some(branch) = branch {
        println!("cargo::rustc-env=DEMI_BUILD_BRANCH={branch}");
    }
}

/// What `git args` prints, trimmed; none when Git is missing or fails, as
/// on a detached HEAD.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
