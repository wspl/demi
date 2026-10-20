//! Shell jobs as the backend runs them (`crates/runner-shell/src/lib.rs`): large
//! output through functions and compound pipelines drains, a kill ends a
//! job that blocks in a utility, and each job runs with the manifest its
//! start names (`crates/runner-jobs/src/commands/mod.rs`).

use std::{collections::BTreeMap, time::Duration};

use demi_runner_protocol::{
    manifest::Manifest,
    wire::{Inbound, JobFileChange, JobPathChange, Outbound, OutputStream, Signal},
};
use serde_json::json;

use demi_command_protocol::CommandCaller;

use crate::{Host, context, start_job, start_job_by};

#[tokio::test]
async fn functions_and_compound_pipelines_drain_large_output_and_here_documents() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        std::fs::write(host.home().join("input"), vec![b'x'; 262_144]).unwrap();
        start_job(
            &mut host,
            "pipeline",
            "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c",
        )
        .await;
        let mut output = Vec::new();
        loop {
            match host.frame().await {
                Outbound::JobOutput { stream, bytes, .. } => {
                    assert_eq!(stream, OutputStream::Stdout, "{:?}", bytes.0);
                    output.extend(bytes.0);
                }
                Outbound::JobExit {
                    exit_code, signal, ..
                } => {
                    assert_eq!(exit_code, Some(0), "{signal:?}");
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output.split_whitespace().collect::<Vec<_>>(),
            ["262144", "262144", "262144", "262145", "262145"]
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_terminates_a_blocking_native_builtin() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        start_job(&mut host, "job", "printf ready; sleep 60").await;
        assert!(matches!(host.frame().await, Outbound::JobOutput { .. }));
        host.send(Inbound::JobKill {
            job_id: "job".into(),
            signal: Some(Signal::Kill),
        })
        .await;
        loop {
            if let Outbound::JobExit { job_id, .. } = host.frame().await {
                assert_eq!(job_id, "job");
                break;
            }
        }
        host.close().await;
    })
    .await
    .unwrap();
}

/// A manifest whose one command is `name`.
fn manifest_of(name: &str) -> Manifest {
    let tree = json!({
        "name": name, "summary": "Fixture command", "kind": "rpc",
        "input": {"type": "object", "properties": {}}
    });
    Manifest::build([serde_json::from_value(tree).unwrap()], []).unwrap()
}

/// Starts `script` as job `job` with the manifest of `hash`.
async fn start_declared(host: &mut Host, job: &str, hash: &str, script: &str) {
    let cwd = host.home().to_string_lossy().into_owned();
    host.send(Inbound::JobStart {
        manifest_hash: Some(hash.into()),
        viewable: None,
        context: context(),
        job_id: job.into(),
        script: script.into(),
        cwd,
        env: BTreeMap::new(),
        stdin: None,
        stdout: None,
    })
    .await;
}

/// A conversation's root and its subagent, whose manifests differ, start a
/// command at the same moment on one Host: the runner installs both, and
/// each job runs with its own, whichever came last. A start naming a
/// manifest the connection never installed fails before its script runs.
/// Cost: one runner process, about a second.
#[tokio::test]
async fn jobs_started_at_once_each_run_with_the_manifest_their_start_names() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let root = manifest_of("root-only");
        let child = manifest_of("child-only");
        for (job, manifest, script) in [
            ("root", &root, "root-only --help >/dev/null && printf root"),
            ("child", &child, "child-only --help >/dev/null && printf child"),
        ] {
            host.send(Inbound::Manifest {
                manifest: serde_json::to_value(manifest).unwrap(),
            })
            .await;
            start_declared(&mut host, job, &manifest.hash, script).await;
        }
        let unknown = manifest_of("never-sent");
        start_declared(&mut host, "unknown", &unknown.hash, "printf ran").await;
        let mut outputs = BTreeMap::<String, Vec<u8>>::new();
        let mut exits = BTreeMap::new();
        while exits.len() < 3 {
            match host.frame().await {
                Outbound::JobOutput { job_id, bytes, .. } => {
                    outputs.entry(job_id).or_default().extend(bytes.0);
                }
                Outbound::JobExit {
                    job_id,
                    exit_code,
                    spawn_error,
                    ..
                } => {
                    exits.insert(job_id, (exit_code, spawn_error.and_then(|error| error.detail)));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(exits["root"], (Some(0), None), "{outputs:?}");
        assert_eq!(exits["child"], (Some(0), None), "{outputs:?}");
        assert_eq!(outputs["root"], b"root");
        assert_eq!(outputs["child"], b"child");
        assert_eq!(
            exits["unknown"],
            (None, Some("job manifest is not installed".to_owned()))
        );
        assert!(!outputs.contains_key("unknown"));
        host.close().await;
    })
    .await
    .unwrap();
}

/// Waits until something is at `path`.
async fn until_exists(path: &std::path::Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A job outlives its connection (`crates/runner/src/connection/kept.rs`): one that
/// ends while the runner is away is listed ended in the next hello, by the
/// same instance, with what it printed meanwhile counted, and its exit
/// follows the hello's answer. About a second: a login shell starts.
#[tokio::test]
async fn a_job_that_ends_while_the_connection_is_lost_is_listed_and_its_exit_follows() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let (instance, kept) = host.kept();
        assert!(kept.is_empty(), "{kept:?}");
        start_job(
            &mut host,
            "kept",
            "printf a; while [ ! -f go ]; do sleep 0.05; done; printf bc; touch done; exit 3",
        )
        .await;
        assert!(matches!(host.frame().await, Outbound::JobOutput { .. }));

        host.cut().await;
        std::fs::write(host.home().join("go"), "").unwrap();
        until_exists(&host.home().join("done")).await;
        host.reconnected().await;

        let (again, kept) = host.kept();
        assert_eq!(again, instance, "the same runner kept its jobs");
        let [job] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!(job.job_id, "kept");
        let ended = job.ended.as_ref().expect("the job ended");
        assert_eq!(ended.exit_code, Some(3));
        assert_eq!(job.output.stdout_bytes, 3);
        let mut host = host.online().await;
        loop {
            if let Outbound::JobExit {
                job_id,
                exit_code,
                output,
                ..
            } = host.frame().await
            {
                assert_eq!((job_id.as_str(), exit_code), ("kept", Some(3)));
                assert_eq!(output.map(|output| output.stdout_bytes), Some(3));
                break;
            }
        }
        // A runner that starts anew draws another instance and keeps none.
        host.restart().await;
        let (other, kept) = host.kept();
        assert_ne!(other, instance);
        assert!(kept.is_empty(), "{kept:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// The runner starts a job id once in its life (`crates/backend-remote-host/src/device_jobs.rs`): a start that arrives again while the job
/// runs leaves it running, once, and one that arrives after its end answers
/// the exit the runner kept, so a start the backend sent again never runs
/// a command twice. About a second: a login shell starts.
#[tokio::test]
async fn a_start_that_arrives_twice_runs_the_job_once() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let script = "printf x >> runs; while [ ! -f go ]; do sleep 0.05; done; exit 4";
        start_job(&mut host, "once", script).await;
        until_exists(&host.home().join("runs")).await;
        start_job(&mut host, "once", script).await;
        std::fs::write(host.home().join("go"), "").unwrap();
        let exit = |frame: Outbound| match frame {
            Outbound::JobExit {
                job_id,
                exit_code,
                spawn_error,
                ..
            } => Some((job_id, exit_code, spawn_error)),
            _ => None,
        };
        let ended = loop {
            if let Some(ended) = exit(host.frame().await) {
                break ended;
            }
        };
        assert_eq!(ended, ("once".to_owned(), Some(4), None));

        start_job(&mut host, "once", script).await;
        let again = loop {
            if let Some(again) = exit(host.frame().await) {
                break again;
            }
        };
        assert_eq!(again, ended, "a start after the end answers the job's exit");

        // Released, whatever the backend had of it, the job still never
        // runs again.
        host.send(Inbound::JobRelease {
            job_id: "once".into(),
        })
        .await;
        start_job(&mut host, "once", script).await;
        let released = loop {
            if let Some(released) = exit(host.frame().await) {
                break released;
            }
        };
        assert_eq!(released.0, "once");
        assert_eq!(std::fs::read_to_string(host.home().join("runs")).unwrap(), "x");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A connection loss stops nothing, however long it lasts; only the
/// backend's refusal of the runner, as for a revoked device, stops its jobs
/// (`crates/runner/src/connection/kept.rs`). The runner's attempts to connect again
/// while it stays away mark the time that passes, so the test waits for
/// them, not for time. About seven seconds: a login shell starts, the
/// runner tries twice in vain and is back within two, and the refused
/// runner takes about five to end.
#[tokio::test]
async fn a_job_runs_on_through_a_connection_loss_until_the_backend_refuses_the_runner() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // The shell creates `pid` before it writes the number, so the job
        // renames it into place: once `pid` exists, it holds the number.
        start_job(
            &mut host,
            "long",
            "sh -c 'echo $$ > pid.new; mv pid.new pid; exec sleep 60'",
        )
        .await;
        let pid = host.home().join("pid");
        until_exists(&pid).await;
        let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        };

        // The job goes on while the runner tries in vain to connect again,
        // and is still running when the runner is back.
        host.cut().await;
        host.drop_next().await;
        host.drop_next().await;
        host.reconnected().await;
        let (_, kept) = host.kept();
        let [job] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!((job.job_id.as_str(), &job.ended), ("long", &None));
        assert!(alive(&pid), "the job {pid:?} runs on");

        // The backend refuses the runner: the runner stops its jobs.
        host.send(Inbound::HelloError {
            code: demi_runner_protocol::wire::HelloErrorCode::UnknownDevice,
            reason: "unknown device".into(),
        })
        .await;
        while alive(&pid) || host.process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

/// Where [`stray_program`] keeps its files, which also starts it.
#[cfg(unix)]
const STRAY_DIRECTORY: &str = "DEMI_TEST_STRAY_DIRECTORY";
/// Set in the child [`stray_program`] starts.
#[cfg(unix)]
const STRAY_CHILD: &str = "DEMI_TEST_STRAY_CHILD";

/// A program the stop tests run in a job, not a test of its own. Started
/// with `STRAY_DIRECTORY` set, it starts itself again and ends once its
/// child has left its group, which the runner kills when its leader ends,
/// so the child is an orphan; the child calls `setsid`, which takes it out of the
/// job's process groups, writes its process id to `pid` in that directory,
/// records each `TERM` it takes in `took-term`, and runs on. This test
/// binary is no Apple program, so macOS shows its environment, where the
/// stop finds it.
#[cfg(unix)]
#[test]
#[ignore = "a program the stop tests run in a job"]
fn stray_program() {
    let Some(directory) = std::env::var_os(STRAY_DIRECTORY).map(std::path::PathBuf::from) else {
        return;
    };
    if std::env::var_os(STRAY_CHILD).is_none() {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["jobs::stray_program", "--exact", "--ignored"])
            .env(STRAY_CHILD, "1")
            .spawn()
            .unwrap();
        while !directory.join("pid").exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    // A child is no group's leader, so it may start a session.
    assert_ne!(unsafe { libc::setsid() }, -1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).unwrap();
        std::fs::write(directory.join("pid.new"), std::process::id().to_string()).unwrap();
        std::fs::rename(directory.join("pid.new"), directory.join("pid")).unwrap();
        while terminate.recv().await.is_some() {
            std::fs::write(directory.join("took-term"), "").unwrap();
        }
    });
}

/// Stops job `job` the way a stop does, by its two signals, while one of its
/// programs that left its groups and writes its process id to `pid` in the
/// runner's home traps `TERM` and runs on: `TERM` reaches the program, and
/// once `KILL` follows, the program has ended by the time the job reports
/// its end (`crates/runner-shell/src/job.rs`).
#[cfg(unix)]
async fn stop_reaches_the_program_that_left(host: &mut Host, job: &str) {
    let pid = host.home().join("pid");
    until_exists(&pid).await;
    let pid = std::fs::read_to_string(&pid).unwrap();
    let running = |pid: &str| {
        let state = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap()
            .stdout;
        // A zombie only waits for its parent to collect it.
        !state.is_empty() && !state.starts_with(b"Z")
    };
    host.send(Inbound::JobKill {
        job_id: job.into(),
        signal: Some(Signal::Terminate),
    })
    .await;
    until_exists(&host.home().join("took-term")).await;
    assert!(running(&pid), "the program traps TERM and runs on");
    host.send(Inbound::JobKill {
        job_id: job.into(),
        signal: Some(Signal::Kill),
    })
    .await;
    loop {
        if let Outbound::JobExit { job_id, signal, .. } = host.frame().await {
            assert_eq!((job_id.as_str(), signal.as_deref()), (job, Some("SIGKILL")));
            break;
        }
    }
    assert!(!running(&pid), "process {pid} survived the stop");
}

/// A stop reaches a program that left the job's process groups and whose
/// parent has ended, as a daemon's does, by the job's id in its environment
/// (`crates/runner-shell/src/job.rs`). The program holds the job's
/// output, so the job runs until `KILL`. Before, neither signal reached it,
/// and it ran on after the job had been reported killed. About a second: a
/// login shell starts.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_reaches_an_orphan_that_left_the_jobs_groups_by_its_environment() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let program = std::env::current_exe().unwrap();
        let script = format!(
            "{STRAY_DIRECTORY}=. '{}' jobs::stray_program --exact --ignored",
            program.display()
        );
        start_job(&mut host, "orphan", &script).await;
        stop_reaches_the_program_that_left(&mut host, "orphan").await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// A stop reaches an Apple program that left the job's process groups while
/// its parent runs, as the parent's descendant (`crates/runner-shell/src/job.rs`): macOS shows no environment of Apple's own programs,
/// such as `/bin/sh` and `perl`. The parent ignores `TERM` and waits. Before,
/// neither signal reached the program. About a second: a login shell starts.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_reaches_a_program_that_left_the_jobs_groups_by_its_parent() {
    const DAEMON: &str = r#"trap '' TERM
perl -e 'use POSIX qw(setsid); setsid(); $SIG{TERM} = sub { open(my $f, ">", "took-term"); close $f; }; open(my $f, ">", "pid.new"); print $f $$; close $f; rename("pid.new", "pid"); sleep 1 while 1;' &
wait
"#;
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        std::fs::write(host.home().join("daemon.sh"), DAEMON).unwrap();
        start_job(&mut host, "daemon", "/bin/sh daemon.sh").await;
        stop_reaches_the_program_that_left(&mut host, "daemon").await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// The files and renames of the next job to exit, `job`.
async fn reported(host: &mut Host, job: &str) -> (Vec<JobFileChange>, Vec<JobPathChange>) {
    loop {
        if let Outbound::JobExit {
            job_id,
            exit_code,
            files,
            path_changes,
            ..
        } = host.frame().await
        {
            assert_eq!((job_id.as_str(), exit_code), (job, Some(0)));
            return (files, path_changes);
        }
    }
}

/// A job's report names what it wrote wherever it is, so a later move out
/// of `/tmp` into the job's working directory carries the file's edits to
/// its new name, which the backend then shows (`crates/agent-store/src/changes.rs`); a file a job made and removed is not listed at
/// all (§ The report). About a second: three jobs start a login shell.
#[tokio::test]
async fn a_move_out_of_tmp_carries_the_files_edits_and_a_made_and_removed_file_is_not_listed() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // `/tmp` itself, which macOS links to `/private/tmp`; the jobs run
        // in the Host's home.
        let scratch = tempfile::tempdir_in("/tmp").unwrap();
        let written = scratch.path().join("x").to_string_lossy().into_owned();
        let moved = std::fs::canonicalize(host.home())
            .unwrap()
            .join("y")
            .to_string_lossy()
            .into_owned();

        start_job(&mut host, "write", &format!("printf 'one\\n' > {written}")).await;
        let (files, _) = reported(&mut host, "write").await;
        let [file] = files.as_slice() else {
            panic!("{files:?}");
        };
        assert_eq!(file.path, written);

        start_job(
            &mut host,
            "move",
            &format!("printf 'two\\n' >> {written}; mv {written} {moved}"),
        )
        .await;
        let (files, path_changes) = reported(&mut host, "move").await;
        let [file] = files.as_slice() else {
            panic!("{files:?}");
        };
        assert_eq!(file.path, moved);
        let [edit] = file.edits.as_slice() else {
            panic!("{:?}", file.edits);
        };
        let side = |path: &Option<String>| std::fs::read_to_string(path.as_ref().unwrap()).unwrap();
        assert_eq!((side(&edit.original), side(&edit.modified)), ("one\n".into(), "one\ntwo\n".into()));
        assert_eq!(
            path_changes,
            [JobPathChange::Renamed {
                from: written.clone(),
                to: moved.clone(),
            }]
        );

        let old = scratch.path().join("old").to_string_lossy().into_owned();
        std::fs::write(&old, "old\n").unwrap();
        let made = scratch.path().join("made").to_string_lossy().into_owned();
        start_job(&mut host, "remove", &format!("rm {old}; printf 'x\n' > {made}; rm {made}")).await;
        let (files, path_changes) = reported(&mut host, "remove").await;
        assert_eq!(files, []);
        assert_eq!(path_changes, [JobPathChange::Removed { path: old.clone() }]);
        host.close().await;
    })
    .await
    .unwrap();
}

/// Each agent of a conversation finds its own scratch directory made and
/// named by `DEMI_SCRATCH` (`crates/runner-jobs/src/scratch.rs`): the root
/// agent's is the conversation's, each subagent's its `agent-<number>`
/// folder inside it, so two subagents that save one relative name keep
/// their own files. Under a second: three short jobs, one after another.
#[tokio::test]
async fn each_agent_finds_its_own_scratch_directory_made_and_named() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let working = tempfile::tempdir_in("/tmp").unwrap();
        let conversation = host.state().join("scratch").join(crate::CONVERSATION);
        for (number, expected) in [
            (0, conversation.clone()),
            (4, conversation.join("agent-4")),
            (5, conversation.join("agent-5")),
        ] {
            let script = format!(
                r#"test -d "$DEMI_SCRATCH" && mkdir -p "$DEMI_SCRATCH/shots" && printf {number} > "$DEMI_SCRATCH/shots/before.png" && printf '%s' "$DEMI_SCRATCH""#
            );
            let job = format!("agent-{number}");
            start_job_by(&mut host, &job, working.path(), CommandCaller::agent(number), &script).await;
            let mut named = Vec::new();
            loop {
                match host.frame().await {
                    Outbound::JobOutput { bytes, .. } => named.extend(bytes.0),
                    Outbound::JobExit { exit_code, .. } => {
                        assert_eq!(exit_code, Some(0), "agent {number}'s directory exists");
                        break;
                    }
                    _ => {}
                }
            }
            assert_eq!(String::from_utf8(named).unwrap(), expected.to_string_lossy());
        }
        for (number, directory) in [
            (0, conversation.clone()),
            (4, conversation.join("agent-4")),
            (5, conversation.join("agent-5")),
        ] {
            let saved = std::fs::read_to_string(directory.join("shots/before.png")).unwrap();
            assert_eq!(saved, number.to_string(), "agent {number}'s file is its own");
        }
        host.close().await;
    })
    .await
    .unwrap();
}

/// A runner that connects removes the scratch directory of a conversation
/// the backend's welcome no longer names, a deleted one's, and keeps those
/// of the conversations it names, and anything under `scratch` that is no
/// directory (`crates/runner-jobs/src/scratch.rs`). About a second: the runner
/// starts twice and runs two jobs.
#[tokio::test]
async fn a_reconnect_removes_the_scratch_directory_of_a_deleted_conversation_only() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // The first welcome names neither conversation, and its cleanup
        // would remove what the test makes next; the runner routes its
        // messages in order and cleans up before it starts a job, so a job's
        // exit says that cleanup is over.
        start_job(&mut host, "before", "true").await;
        reported(&mut host, "before").await;
        let root = host.state().join("scratch");
        for conversation in ["kept", "deleted"] {
            std::fs::create_dir_all(root.join(conversation)).unwrap();
            std::fs::write(root.join(conversation).join("page.png"), "png").unwrap();
        }
        std::fs::write(root.join("notes"), "not a conversation's").unwrap();

        host.restart().await;
        host.welcome(&["kept", crate::CONVERSATION]).await;
        // The runner removes them before it starts the next job.
        start_job(&mut host, "after", "true").await;
        reported(&mut host, "after").await;
        assert!(!root.join("deleted").exists());
        assert!(root.join("kept").join("page.png").exists());
        assert!(root.join("notes").exists());
        host.close().await;
    })
    .await
    .unwrap();
}
