use demi_runner_protocol::wire::{self, Inbound};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

async fn call<T: serde::de::DeserializeOwned>(message: Inbound, root: &std::path::Path) -> T {
    let bytes = demi_runner_host::fs::handle(&message, root, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    rmp_serde::from_slice::<Reply<T>>(&bytes).unwrap().result
}

#[tokio::test]
async fn filesystem_wire_preserves_binary_dates_links_and_error_codes() {
    let root = tempfile::tempdir().unwrap();
    let id = "test".to_owned();
    // File contents travel through pipes (`demi_runner_host::files`); the file is
    // set up on disk and the metadata requests are checked against it.
    std::fs::create_dir(root.path().join("nested")).unwrap();
    std::fs::write(root.path().join("nested/a"), [0, 255, 128, 10]).unwrap();
    assert!(
        call::<wire::FileStat>(
            Inbound::FsStat {
                id: id.clone(),
                path: "a".into(),
                cwd: Some(root.path().join("nested").to_string_lossy().into_owned())
            },
            root.path()
        )
        .await
        .is_file
    );
    call::<()>(
        Inbound::FsUtimes {
            id: id.clone(),
            path: "nested/a".into(),
            cwd: None,
            atime: wire::Timestamp(1234567890123),
            mtime: wire::Timestamp(1234567890123),
        },
        root.path(),
    )
    .await;
    let stat = call::<wire::FileStat>(
        Inbound::FsStat {
            id: id.clone(),
            path: "nested/a".into(),
            cwd: None,
        },
        root.path(),
    )
    .await;
    assert_eq!(stat.mtime.0, 1234567890123);
    assert!(stat.is_file);
    assert_eq!(stat.size, 4);
    call::<()>(
        Inbound::FsLink {
            id: id.clone(),
            path: "hard".into(),
            cwd: None,
            existing_path: "nested/a".into(),
        },
        root.path(),
    )
    .await;
    #[cfg(unix)]
    {
        call::<()>(
            Inbound::FsSymlink {
                id: id.clone(),
                path: "symbolic".into(),
                cwd: None,
                target: "nested/a".into(),
            },
            root.path(),
        )
        .await;
        assert_eq!(
            call::<String>(
                Inbound::FsReadlink {
                    id: id.clone(),
                    path: "symbolic".into(),
                    cwd: None
                },
                root.path()
            )
            .await,
            "nested/a"
        );
        assert!(
            call::<wire::FileStat>(
                Inbound::FsLstat {
                    id: id.clone(),
                    path: "symbolic".into(),
                    cwd: None
                },
                root.path()
            )
            .await
            .is_symbolic_link
        );
    }
    call::<()>(
        Inbound::FsCp {
            id: id.clone(),
            path: "nested".into(),
            cwd: None,
            destination: "copied".into(),
            recursive: Some(true),
        },
        root.path(),
    )
    .await;
    call::<()>(
        Inbound::FsMv {
            id: id.clone(),
            path: "copied".into(),
            cwd: None,
            destination: "moved".into(),
        },
        root.path(),
    )
    .await;
    assert_eq!(
        std::fs::read(root.path().join("moved/a")).unwrap(),
        [0, 255, 128, 10]
    );
    call::<()>(
        Inbound::FsRm {
            id: id.clone(),
            path: "moved".into(),
            cwd: None,
            recursive: Some(true),
            force: None,
        },
        root.path(),
    )
    .await;
    let missing = Inbound::FsStat {
        id,
        path: "missing".into(),
        cwd: None,
    };
    let bytes = demi_runner_host::fs::handle(&missing, root.path(), &CancellationToken::new())
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    let error: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(error["type"], "fs_error");
    assert_eq!(error["code"], "ENOENT");
}

/// A listing names each symbolic link's target as written and what is
/// there after every link on the way, and a link that leads nowhere is
/// `missing`, never a failure of the listing (`runner.md` § Host
/// operations).
// Cost: a temporary directory; a few milliseconds.
#[cfg(unix)]
#[tokio::test]
async fn a_listing_says_where_each_link_points_and_never_fails_for_one() {
    use std::os::unix::fs::symlink;
    use wire::LinkTargetKind::{Directory, File, Missing};

    let root = tempfile::tempdir().unwrap();
    let at = |name: &str| root.path().join(name);
    std::fs::write(at("notes.txt"), "four").unwrap();
    std::fs::create_dir(at("shared")).unwrap();
    symlink("notes.txt", at("to-file")).unwrap();
    symlink("shared", at("to-dir")).unwrap();
    let gone = at("gone.log").to_string_lossy().into_owned();
    symlink(&gone, at("broken")).unwrap();
    symlink("loop-b", at("loop-a")).unwrap();
    symlink("loop-a", at("loop-b")).unwrap();
    symlink("to-dir", at("chain")).unwrap();

    let mut entries = call::<Vec<wire::DirEntry>>(
        Inbound::FsReaddir {
            id: "test".into(),
            path: ".".into(),
            cwd: None,
        },
        root.path(),
    )
    .await;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let listed: Vec<_> = entries
        .iter()
        .map(|entry| {
            let link = entry.link.as_ref().map(|link| (link.target.as_str(), link.target_kind));
            (entry.name.as_str(), entry.is_file, entry.is_directory, link)
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("broken", false, false, Some((gone.as_str(), Missing))),
            ("chain", false, false, Some(("to-dir", Directory))),
            ("loop-a", false, false, Some(("loop-b", Missing))),
            ("loop-b", false, false, Some(("loop-a", Missing))),
            ("notes.txt", true, false, None),
            ("shared", false, true, None),
            ("to-dir", false, false, Some(("shared", Directory))),
            ("to-file", false, false, Some(("notes.txt", File))),
        ]
    );
}
