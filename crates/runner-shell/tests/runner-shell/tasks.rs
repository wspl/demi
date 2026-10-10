//! A job's background tasks and `timeout` (`runner.md` § Background tasks
//! and timeouts): `$!` names a task by an id above every process ID, which
//! `kill`, `wait` and `jobs -p` take; `kill` stops the whole task, and
//! `wait` gives bash's statuses; `timeout` stops its command as `kill` stops
//! a task, with GNU's statuses; `disown` and `nohup` keep a task in its job,
//! which names the tasks that outlive its script.

use demi_runner_process::job_shell::ShellJob;
use demi_runner_protocol::wire::Signal;
use demi_runner_shell::{
    ShellRuntime,
    testing::{Job, Scope, ShellOptions, execute},
};
use std::{fs, path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

/// Runs `script` as a job in `root`, with the system's programs on `PATH`,
/// and returns its status, standard output and standard error.
async fn job(root: &Path, script: &str) -> (u8, String, String) {
    let output = tempfile::NamedTempFile::new().unwrap();
    let error = tempfile::NamedTempFile::new().unwrap();
    let mut env = super::home(root);
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        execute(
            script,
            ShellOptions {
                scope: Scope::new(CancellationToken::new(), None),
                login: false,
                cwd: root.into(),
                env,
                stdin: tempfile::tempfile().unwrap(),
                stdout: output.reopen().unwrap(),
                stderr: error.reopen().unwrap(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    (
        result.code,
        fs::read_to_string(output.path()).unwrap(),
        fs::read_to_string(error.path()).unwrap(),
    )
}

/// Waits until `pid` is gone; a hang guard of 60 s.
async fn gone(pid: i32) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("process {pid} survived"));
}

/// `$!` names each task by the job's next id from 4194305, `jobs -p` lists
/// them, and `kill` and `wait` take them: `wait` gives the task's last
/// status, or 128 plus the signal that ended it, and alone waits for every
/// task and gives 0. Before, `$!` was empty, `kill` defaulted to `KILL` and
/// `wait` refused process IDs. About 0.1 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_tasks_have_ids_that_kill_and_wait_take() {
    let root = tempfile::tempdir().unwrap();
    let script = "/bin/sleep 30 & p=$!; echo \"first $p\"; jobs -p; \
                  kill $p; wait $p; echo \"term $?\"; \
                  /bin/sleep 30 & echo \"second $!\"; kill -9 $!; wait $!; echo \"kill $?\"; \
                  /bin/sleep 30 & kill -s INT $!; wait $!; echo \"int $?\"; \
                  (exit 3) & wait $!; echo \"last $?\"; \
                  sleep 30 & kill %+; wait %+; echo \"spec $?\"; \
                  (exit 4) & wait; echo \"all $?\"; \
                  wait $p; echo \"again $?\"";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(
        output,
        "first 4194305\n4194305\nterm 143\nsecond 4194306\nkill 137\nint 130\nlast 3\n\
         spec 143\nall 0\nagain 127\n"
    );
    assert_eq!(errors, "wait: pid 4194305 is not a child of this shell\n");
}

/// `kill` on a task stops the subshell and everything it started: the
/// program the subshell's program started in the background is gone too,
/// where bash's `kill` would signal the subshell alone. A process ID or a
/// process group's negative ID signals that process or group, after `--` or
/// not. Before, `$!` was empty and `kill -- -N` read `-N` as a signal.
/// About 0.1 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_stops_a_task_with_everything_it_started() {
    let root = tempfile::tempdir().unwrap();
    let script = "(/bin/sh -c '/bin/sleep 30 & echo $! > child; wait') & task=$!; \
                  until [ -s child ]; do sleep 0.01; done; \
                  kill $task; wait $task; echo \"task $?\"; \
                  /bin/sh -c 'echo $$ > leader; exec /bin/sleep 30' & \
                  until [ -s leader ]; do sleep 0.01; done; \
                  kill -- -$(cat leader); wait $!; echo \"group $?\"; \
                  /bin/sh -c 'echo $$ > single; exec /bin/sleep 30' & \
                  until [ -s single ]; do sleep 0.01; done; \
                  kill -TERM $(cat single); wait $!; echo \"process $?\"";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "task 143\ngroup 143\nprocess 143\n");
    let child = fs::read_to_string(root.path().join("child")).unwrap();
    gone(child.trim().parse().unwrap()).await;
}

/// A task that ignores `TERM` runs on; `KILL` ends it, and `wait` gives
/// 137. About 0.1 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_dash_nine_ends_a_task_that_ignores_term() {
    let root = tempfile::tempdir().unwrap();
    let script = "/bin/sh -c 'trap \"echo ignored > ignored\" TERM; echo $$ > leader; \
                  while :; do /bin/sleep 1 & wait; done' & task=$!; \
                  until [ -s leader ]; do sleep 0.01; done; \
                  kill $task; until [ -s ignored ]; do sleep 0.01; done; \
                  kill -0 $task; echo \"alive $?\"; \
                  kill -KILL $task; wait $task; echo \"killed $?\"";
    let (code, output, errors) = job(root.path(), script).await;
    assert_eq!(code, 0, "{errors}");
    assert_eq!(output, "alive 0\nkilled 137\n");
    let leader = fs::read_to_string(root.path().join("leader")).unwrap();
    gone(leader.trim().parse().unwrap()).await;
}

/// `timeout` runs a builtin, a standard utility or a program and stops it
/// when its time runs out, with GNU's options, messages and statuses, from
/// the job's shell and from a utility that starts it. Before, `timeout` was
/// the system's program, or none. About 0.6 s: each case that times out
/// waits a tenth of a second for it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_stops_its_command_with_gnus_statuses() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("afile"), "text\n").unwrap();
    let cases: &[(&str, &str, &str)] = &[
        ("timeout 0.1 sleep 5", "124", ""),
        ("timeout 0.1 /bin/sleep 5", "124", ""),
        ("timeout 5 echo hi", "hi\n0", ""),
        ("timeout -s KILL 0.1 /bin/sleep 5", "137", ""),
        ("timeout --preserve-status 0.1 /bin/sleep 5", "143", ""),
        (
            "timeout -k 0.1 0.1 /bin/sh -c 'trap \"\" TERM; while :; do :; done'",
            "137",
            "",
        ),
        (
            "timeout -v 0.1 sleep 5",
            "124",
            "timeout: sending signal TERM to command ‘sleep’\n",
        ),
        ("timeout 5 /bin/sh -c 'exit 3'", "3", ""),
        (
            "timeout abc sleep 1",
            "125",
            "timeout: invalid time interval ‘abc’\nTry 'timeout --help' for more information.\n",
        ),
        (
            "timeout -s FOO 1 sleep 1",
            "125",
            "timeout: ‘FOO’: invalid signal\nTry 'timeout --help' for more information.\n",
        ),
        (
            "timeout 1 nosuch",
            "127",
            "timeout: failed to run command ‘nosuch’: No such file or directory\n",
        ),
        (
            "timeout 1 ./afile",
            "126",
            "timeout: failed to run command ‘./afile’: Permission denied\n",
        ),
        ("echo 5 | xargs timeout 0.1 sleep", "123", ""),
        ("echo x | xargs timeout 5 echo", "x\n0", ""),
    ];
    for (script, output, errors) in cases {
        let (_, printed, printed_errors) = job(root.path(), &format!("{script}; echo $?")).await;
        assert_eq!(
            (printed.as_str(), printed_errors.as_str()),
            (&*format!("{output}\n"), *errors),
            "{script}"
        );
    }
}

/// Starts `script` as the runner starts a job, with the system's programs
/// on `PATH`, reads its output until `ready`, waits until the job names the
/// tasks that keep it running after its script, and stops it with `TERM`;
/// a task that runs on keeps the job from ending within the hang guard.
/// Gives what it printed and those tasks; the process whose ID the script
/// wrote to `leader` is gone.
#[cfg(unix)]
async fn stopped_after_its_script(script: &str, ready: &str) -> (String, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let mut env = super::home(root.path());
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    let mut job = Job::start(
        script.into(),
        root.path().into(),
        env,
        false,
        Scope::new(CancellationToken::new(), None),
        &ShellRuntime::current(),
    )
    .await
    .unwrap();
    let mut read = String::new();
    crate::jobs::output_until(&mut job, &mut read, ready).await;
    let mut outliving = job.outliving();
    let outliving = tokio::time::timeout(
        Duration::from_secs(60),
        outliving.wait_for(|tasks| !tasks.is_empty()),
    )
    .await
    .expect("the job names the tasks that outlive its script")
    .unwrap()
    .clone();
    job.signal(Signal::Terminate).unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(60), job.wait())
        .await
        .unwrap();
    assert_eq!(exit.signal.as_deref(), Some("SIGTERM"));
    // The job ends once every process it started has.
    let leader: i32 = fs::read_to_string(root.path().join("leader")).unwrap().trim().parse().unwrap();
    assert_eq!(unsafe { libc::kill(leader, 0) }, -1, "{leader} survived");
    (read, outliving)
}

/// `disown` keeps a task in its job: it succeeds, `jobs -p` still lists the
/// task, the job runs on after its script while the task does and names it,
/// and stopping the job stops the task, so no process is left. Before,
/// `disown` was brush's unimplemented builtin and gave 99. About 0.05 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disown_keeps_a_task_in_its_job() {
    let task = "/bin/sh -c 'echo $$ > leader; exec /bin/sleep 300'";
    let script = format!(
        "{task} & disown; echo \"disown $?\"; jobs -p; \
         until [ -s leader ]; do sleep 0.01; done; echo ready"
    );
    let (read, outliving) = stopped_after_its_script(&script, "ready\n").await;
    assert_eq!(read, "disown 0\n4194305\nready\n");
    assert_eq!(outliving, [task]);
}

/// `nohup` runs its command ignoring `HUP`, as a process of the job: the
/// command survives the `HUP` it sends itself, the job runs on after its
/// script while the command does and names it, and stopping the job stops
/// the command, so no process is left. The Host's own `nohup` does this; a
/// command that started a session of its own would run on past the stop
/// and hold the job until the hang guard. About 0.05 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nohup_keeps_a_task_in_its_job_and_ignores_hup() {
    let task = "nohup /bin/sh -c 'echo $$ > leader; kill -HUP $$; echo survived; exec /bin/sleep 300'";
    let script = format!("{task} & echo started");
    let (read, outliving) = stopped_after_its_script(&script, "survived\n").await;
    assert_eq!(read, "started\nsurvived\n");
    assert_eq!(outliving, [task]);
}
