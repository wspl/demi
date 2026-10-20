//! A look within a directory follows no link below it
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
// Cost: a temporary directory; a few milliseconds.

/// A project whose `.demi/workflows/review.yaml` is a file, beside a
/// link at each level a workflow read passes.
#[cfg(unix)]
#[test]
fn no_link_below_the_directory_is_followed_and_a_file_reads_up_to_its_limit() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(project.join(".demi/workflows")).unwrap();
    std::fs::create_dir_all(outside.join("workflows")).unwrap();
    std::fs::write(project.join(".demi/workflows/review.yaml"), "name: Review").unwrap();
    std::fs::write(outside.join("workflows/review.yaml"), "name: Outside").unwrap();
    std::os::unix::fs::symlink(outside.join("workflows/review.yaml"), project.join(".demi/workflows/linked.yaml")).unwrap();
    let look = |path: &std::path::Path, limit| {
        let mut bytes = Vec::new();
        let looked = demi_runner_host::nofollow::look_within(&project, path, limit, &mut bytes).unwrap();
        (looked, String::from_utf8(bytes).unwrap())
    };

    assert_eq!(look(&project.join(".demi/workflows/review.yaml"), 4), (demi_runner_protocol::wire::Looked::File { size: 12, read: 4 }, "name".to_owned()));
    assert_eq!(look(&project.join(".demi/workflows/linked.yaml"), 100), (demi_runner_protocol::wire::Looked::Link, String::new()));
    assert_eq!(look(&project.join(".demi/workflows/gone.yaml"), 100), (demi_runner_protocol::wire::Looked::Missing, String::new()));
    let (listed, _) = look(&project.join(".demi/workflows"), 0);
    let demi_runner_protocol::wire::Looked::Directory { mut entries } = listed else { panic!("{listed:?}") };
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        entries.iter().map(|entry| (entry.name.as_str(), entry.link.clone())).collect::<Vec<_>>(),
        [
            (
                "linked.yaml",
                Some(demi_runner_protocol::wire::SymbolicLink {
                    target: outside.join("workflows/review.yaml").to_string_lossy().into_owned(),
                    leads_to: demi_runner_protocol::wire::LinkTarget::File,
                })
            ),
            ("review.yaml", None)
        ]
    );

    // `.demi` swapped for a link to a directory that holds the same
    // names: neither it nor anything behind it is opened.
    std::fs::rename(project.join(".demi"), root.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, project.join(".demi")).unwrap();
    assert_eq!(look(&project.join(".demi"), 0).0, demi_runner_protocol::wire::Looked::Link);
    assert_eq!(look(&project.join(".demi/workflows"), 0).0, demi_runner_protocol::wire::Looked::Link);
    assert_eq!(look(&project.join(".demi/workflows/review.yaml"), 100), (demi_runner_protocol::wire::Looked::Link, String::new()));

    // A link above the directory is the project's own business.
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&project, &alias).unwrap();
    let mut bytes = Vec::new();
    assert_eq!(demi_runner_host::nofollow::look_within(&alias, &alias.join(".demi"), 0, &mut bytes).unwrap(), demi_runner_protocol::wire::Looked::Link);
    assert!(demi_runner_host::nofollow::look_within(&project, &root.path().join("moved"), 0, &mut bytes).is_err());
}
