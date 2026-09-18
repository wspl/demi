use demi_runner::connection::wire::{self as wire, Inbound};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

async fn call<T: serde::de::DeserializeOwned>(message: Inbound, root: &std::path::Path) -> T {
    let bytes = demi_runner::fs::handle(&message, root, &CancellationToken::new())
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
    // File contents travel through pipes (`demi_runner::files`); the file is
    // set up on disk and the metadata requests are checked against it.
    std::fs::create_dir(root.path().join("nested")).unwrap();
    std::fs::write(root.path().join("nested/a"), [0, 255, 128, 10]).unwrap();
    assert!(
        call::<wire::FsOkStatResult>(
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
    let stat = call::<wire::FsOkStatResult>(
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
    assert_eq!(stat.size, 4.);
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
            call::<wire::FsOkStatResult>(
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
    let bytes = demi_runner::fs::handle(&missing, root.path(), &CancellationToken::new())
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    let error: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(error["type"], "fs_error");
    assert_eq!(error["code"], "ENOENT");
}

#[tokio::test]
async fn a_reply_over_the_message_limit_fails_its_request() {
    let root = tempfile::tempdir().unwrap();
    // Long names make a listing outgrow the limit with a few thousand entries.
    let name = "x".repeat(200);
    let count = wire::MAX_MESSAGE_BYTES / 200 + 1;
    for index in 0..count {
        std::fs::File::create(root.path().join(format!("{name}{index}"))).unwrap();
    }
    let listing = Inbound::FsReaddir {
        id: "big".into(),
        path: ".".into(),
        cwd: None,
        with_file_types: None,
    };
    let bytes = demi_runner::fs::handle(&listing, root.path(), &CancellationToken::new())
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    assert!(bytes.len() <= wire::MAX_MESSAGE_BYTES);
    let error: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(error["type"], "fs_error");
    assert_eq!(error["id"], "big");
    assert_eq!(error["code"], "too_large");
}
