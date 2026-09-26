use demi_runner::shell::{ShellOptions, execute};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

fn options(root: &Path, output: std::fs::File) -> ShellOptions {
    ShellOptions {
        scope: demi_runner::shell::scope::Scope::new(
            tokio_util::sync::CancellationToken::new(),
            None,
        ),
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

/// `wc -c` counts a file whose size is a whole number of pages whole: it
/// seeks near the end, and inside a job it reads the rest, since a job's
/// utilities never splice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wc_counts_a_file_of_whole_pages_whole() {
    let root = tempfile::tempdir().unwrap();
    // Whole pages for page sizes of 4, 16 and 64 KiB.
    std::fs::write(root.path().join("pages.bin"), vec![7_u8; 5 * 65536]).unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let result = execute("wc -c pages.bin", options(root.path(), output.reopen().unwrap()))
        .await
        .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(
        std::fs::read_to_string(output.path()).unwrap().trim(),
        "327680 pages.bin"
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
                demi_runner::commands::command_client::CONTEXT_ENV.into(),
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
    let output = tempfile::NamedTempFile::new().unwrap();
    let mut opts = options(root.path(), output.reopen().unwrap());
    opts.env = BTreeMap::from([
        ("HOME".into(), root.path().to_string_lossy().into_owned()),
        ("DRIVE_ROOT".into(), drive_path(root.path())),
        (
            "DRIVE_EXE".into(),
            drive_path(&Path::new(&std::env::var("SystemRoot").unwrap()).join("System32/cmd.exe")),
        ),
    ]);
    let result = execute(
        "printf discarded > /dev/null && printf discarded &> /dev/null && printf payload > \"$DRIVE_ROOT/file\" && cd \"$DRIVE_ROOT/sub\" && cat \"$DRIVE_ROOT/file\" && \"$DRIVE_EXE\" /c \"echo external\"",
        opts,
    )
    .await
    .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(result.cwd, root.path().join("sub"));
    assert_eq!(
        std::fs::read_to_string(output.path()).unwrap(),
        "payloadexternal\r\n"
    );
}

/// Runs `script` as a job in `root`: its status, output and errors.
async fn job(root: &Path, script: &str) -> (u8, String, String) {
    let output = tempfile::NamedTempFile::new().unwrap();
    let errors = tempfile::NamedTempFile::new().unwrap();
    let mut options = options(root, output.reopen().unwrap());
    options.stderr = errors.reopen().unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), execute(script, options))
        .await
        .unwrap()
        .unwrap();
    let read = |file: &tempfile::NamedTempFile| std::fs::read_to_string(file.path()).unwrap();
    (result.code, read(&output), read(&errors))
}

/// A job's `exec` runs its command as the shell's last and ends the shell
/// with the command's status; in a subshell it ends the subshell. Bash's
/// would replace the process, which is the runner: before, this test's own
/// process became the command.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_ends_the_shell_with_its_command_and_leaves_the_runner() {
    let root = tempfile::tempdir().unwrap();
    let script = "(exec /bin/sh -c 'exit 3'); echo \"subshell $?\"; \
                  exec /bin/sh -c 'echo last; exit 7'; echo after";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 7, "{errors}");
    assert_eq!(output, "subshell 3\nlast\n");
}

/// A job's `ulimit` holds for the processes its shell starts from then on,
/// not for the runner, whose limits every job shares, nor for another job;
/// a limit the system refuses fails and changes nothing. Before, a job's
/// `ulimit -n` set this test's own process's limit.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ulimit_limits_the_jobs_own_processes_only() {
    let root = tempfile::tempdir().unwrap();
    let runner = rlimit::getrlimit(rlimit::Resource::NOFILE).unwrap();
    let script = "ulimit -n 64; ulimit -n; /bin/sh -c 'ulimit -n'; \
                  ulimit -n 99999999999; echo \"refused $?\"; ulimit -Hn";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "64\n64\nrefused 1\n64\n");
    assert!(errors.contains("open files: cannot modify limit"), "{errors}");
    assert_eq!(rlimit::getrlimit(rlimit::Resource::NOFILE).unwrap(), runner);
    let (_, output, _) = job(root.path(), "ulimit -n; /bin/sh -c 'ulimit -n'").await;
    let (soft, _) = demi_runner::process::child_limit(rlimit::Resource::NOFILE).unwrap();
    assert_eq!(output, format!("{soft}\n{soft}\n"));
}

/// A job's `umask` holds for the processes its shell starts and the files
/// its redirections and standard utilities create, not for the runner nor
/// another job. Before, it set this test's own process's umask.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn umask_masks_the_jobs_own_processes_and_files_only() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let mode = |name: &str| {
        let metadata = std::fs::metadata(root.path().join(name)).unwrap();
        metadata.permissions().mode() & 0o777
    };
    std::fs::write(root.path().join("runner-before"), "").unwrap();
    let script = "umask 077; umask; /bin/sh -c umask; umask -S; \
                  echo x > redirected; touch touched; mkdir made";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "0077\n0077\nu=rwx,g=,o=\n");
    assert_eq!(mode("redirected"), 0o600);
    assert_eq!(mode("touched"), 0o600);
    assert_eq!(mode("made"), 0o700);
    std::fs::write(root.path().join("runner-after"), "").unwrap();
    assert_eq!(mode("runner-after"), mode("runner-before"));
    let (_, output, _) = job(root.path(), "umask; /bin/sh -c umask").await;
    let runner = format!("{:04o}\n", demi_runner::process::umask());
    assert_eq!(output, format!("{runner}{runner}"));
}

/// A job's `kill` cannot signal the runner: `$$` names the runner's process
/// and 0 its process group. Before, `kill $$` killed this test's own
/// process.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_refuses_the_runner() {
    let root = tempfile::tempdir().unwrap();
    let script = "kill $$; echo \"runner $?\"; kill -s TERM 0; echo \"group $?\"";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "runner 1\ngroup 1\n");
    assert_eq!(errors.matches("a job cannot signal the runner it runs in").count(), 2, "{errors}");
}

/// A job's `suspend` would stop the runner; it refuses. Before, it stopped
/// this test's own process.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspend_refuses() {
    let root = tempfile::tempdir().unwrap();
    let (code, output, errors) = job(root.path(), "suspend -f; echo \"suspend $?\"").await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "suspend 1\n");
    assert!(errors.contains("a job cannot suspend the runner it runs in"), "{errors}");
}

/// A job has no job control, as a bash script has none: `fg` would give the
/// runner's terminal to one of its processes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fg_refuses_without_job_control() {
    let root = tempfile::tempdir().unwrap();
    let (code, output, errors) = job(root.path(), "true & fg; echo \"fg $?\"").await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "fg 1\n");
    assert!(errors.contains("fg: no job control"), "{errors}");
}
