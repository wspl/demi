use demi_runner::shell::{ShellOptions, execute};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

fn options(root: &Path, output: std::fs::File) -> ShellOptions {
    ShellOptions {
        login: false,
        cwd: root.into(),
        env: BTreeMap::new(),
        stdin: tempfile::tempfile().unwrap(),
        stdout: output,
        stderr: tempfile::tempfile().unwrap(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn builtin_pipeline_emits_before_input_eof() {
    let root = tempfile::tempdir().unwrap();
    let (input, mut writer) = std::io::pipe().unwrap();
    let (mut reader, output) = std::io::pipe().unwrap();
    #[cfg(unix)]
    use std::os::fd::OwnedFd as Owned;
    #[cfg(windows)]
    use std::os::windows::io::OwnedHandle as Owned;
    let mut opts = options(root.path(), Owned::from(output).into());
    opts.stdin = Owned::from(input).into();
    let job = tokio::spawn(async move { execute("cat | cat", opts).await.unwrap() });
    writer.write_all(&[0, 255, 128, 10]).unwrap();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes).unwrap();
            bytes
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(bytes, [0, 255, 128, 10]);
    drop(writer);
    assert_eq!(job.await.unwrap().code, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_handles_redirects_functions_subshell_cwd_and_fresh_state() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let script = "mkdir a b; f() { printf '%s\\n' \"$1\"; }; f pear > a/input; (cd a; cat input) | tr a-z A-Z; cd b; export LEAK=bad";
    let result = execute(script, options(root.path(), output.reopen().unwrap()))
        .await
        .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(result.cwd, root.path().join("b"));
    assert_eq!(std::fs::read_to_string(output.path()).unwrap(), "PEAR\n");
    let output = tempfile::NamedTempFile::new().unwrap();
    let result = execute(
        "printf '%s' \"${LEAK-unset}\"",
        options(&result.cwd, output.reopen().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(std::fs::read_to_string(output.path()).unwrap(), "unset");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tee_and_od_use_pipeline_streams() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        execute(
            "printf hello | tee made.txt | grep hello | od -An -tx1",
            options(root.path(), output.reopen().unwrap()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(
        std::fs::read_to_string(root.path().join("made.txt")).unwrap(),
        "hello"
    );
    assert_eq!(
        std::fs::read_to_string(output.path()).unwrap().trim(),
        "68 65 6c 6c 6f 0a"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_joins_background_tasks_and_preserves_foreground_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let result = execute(
        "(sleep 0.05; echo completed > done) & exit 7",
        options(root.path(), output.reopen().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(result.code, 7);
    assert_eq!(
        std::fs::read_to_string(root.path().join("done")).unwrap(),
        "completed\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_profiles_apply_per_job_without_replacing_owned_context_or_cwd() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_owned();
    std::fs::create_dir(root.join("elsewhere")).unwrap();
    let aliases = root.join("aliases");
    let tools = root.join("tools");
    let profile = root.join(".bash_profile");
    for value in ["first", "next"] {
        std::fs::write(&profile, format!(
            "export FROM_PROFILE={value}\nexport PATH=\"$HOME/tools\"\nexport DEMI_CONTEXT_ID=wrong\ncd \"$HOME/elsewhere\"\n"
        )).unwrap();
        let output = tempfile::NamedTempFile::new().unwrap();
        let mut opts = options(&root, output.reopen().unwrap());
        opts.login = true;
        opts.env = BTreeMap::from([
            ("HOME".into(), root.to_string_lossy().into_owned()),
            ("USERPROFILE".into(), root.to_string_lossy().into_owned()),
            ("PATH".into(), aliases.to_string_lossy().into_owned()),
            (
                demi_runner::command_client::CONTEXT_ENV.into(),
                "owned".into(),
            ),
        ]);
        let result = execute(
            "printf '%s\\n' \"$FROM_PROFILE\" \"$DEMI_CONTEXT_ID\" \"$PATH\"",
            opts,
        )
        .await
        .unwrap();
        assert_eq!(result.code, 0);
        assert_eq!(result.cwd, root);
        let output = std::fs::read_to_string(output.path()).unwrap();
        let mut lines = output.lines();
        assert_eq!(lines.next(), Some(value));
        assert_eq!(lines.next(), Some("owned"));
        let paths = std::env::split_paths(lines.next().unwrap()).collect::<Vec<_>>();
        assert_eq!(paths, [aliases.clone(), tools.clone()]);
        assert_eq!(lines.next(), None);
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_drive_paths_work_for_cd_redirection_utilities_and_executables() {
    fn drive_path(path: &Path) -> String {
        let native = path.to_string_lossy().replace('\\', "/");
        assert_eq!(&native[1..3], ":/");
        format!("/{}/{}", &native[..1], &native[3..])
    }

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    std::fs::write(
        root.path().join("child.json"),
        serde_json::to_vec(&demi_runner::tasks::ShellJob {
            login: false,
            live: false,
            script: "printf external".into(),
            cwd_file: root.path().join("child-cwd"),
        })
        .unwrap(),
    )
    .unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let mut opts = options(root.path(), output.reopen().unwrap());
    opts.env = BTreeMap::from([
        ("HOME".into(), root.path().to_string_lossy().into_owned()),
        ("DRIVE_ROOT".into(), drive_path(root.path())),
        (
            "DRIVE_EXE".into(),
            drive_path(Path::new(env!("CARGO_BIN_EXE_demi-runner"))),
        ),
    ]);
    let result = execute(
        "printf discarded > /dev/null && printf discarded &> /dev/null && printf payload > \"$DRIVE_ROOT/file\" && cd \"$DRIVE_ROOT/sub\" && cat \"$DRIVE_ROOT/file\" && \"$DRIVE_EXE\" shell-job \"$HOME/child.json\"",
        opts,
    )
    .await
    .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(result.cwd, root.path().join("sub"));
    assert_eq!(
        std::fs::read_to_string(output.path()).unwrap(),
        "payloadexternal"
    );
}
