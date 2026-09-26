use demi_command_service::{edits::Recorder, protocol::EditContext};
use demi_runner::shell::{ShellOptions, execute, scope::Scope};
use std::{collections::BTreeMap, fs, path::Path};
use tokio_util::sync::CancellationToken;

async fn run(root: &Path, recorder: Recorder, script: &str) {
    let mut scope = Scope::new(CancellationToken::new(), None);
    scope.edits = Some(recorder);
    let result = execute(
        script,
        ShellOptions {
            scope: scope.clone(),
            login: false,
            cwd: root.to_owned(),
            env: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                // `mktemp` makes its file in the job's own temporary
                // directory, which goes with the test's.
                ("TMPDIR".into(), root.to_string_lossy().into_owned()),
            ]),
            stdin: tempfile::tempfile().unwrap(),
            stdout: tempfile::tempfile().unwrap(),
            stderr: tempfile::tempfile().unwrap(),
        },
    )
    .await
    .unwrap();
    scope.finish().await;
    assert_eq!(result.code, 0);
}

fn recorder(root: &Path, job: &str) -> Recorder {
    Recorder::new(EditContext {
        directory: root.join(job).to_string_lossy().into_owned(),
        lock: root.join("edits.lock").to_string_lossy().into_owned(),
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirections_descriptors_and_utilities_record_actual_contents() {
    let root = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    fs::write(root.path().join("sorted"), "pear\napple\n").unwrap();
    fs::write(root.path().join("restored"), "same\n").unwrap();
    run(
        root.path(),
        tracking.clone(),
        concat!(
            "printf 'one\\n' > file; ",
            "printf 'changed\\n' > restored; printf 'same\\n' > restored; ",
            "exec 3>>file; printf 'two\\n' >&3; exec 3>&-; ",
            "printf 'tea\\n' | tee tee-file >/dev/null; ",
            "sort sorted -o sorted; ",
            "sed -i 's/one/first/' file; ",
            "printf 'same\\nsame\\n' | uniq - unique; ",
            "printf temporary > temporary; rm temporary; ",
            "cp file copy; mv copy moved; touch touched; mktemp >/dev/null",
        ),
    )
    .await;
    let files = tracking.report().unwrap().files;
    let names: Vec<_> = files
        .iter()
        .map(|file| Path::new(&file.path).file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["file", "tee-file", "sorted", "unique"]);
    for (name, expected) in [
        ("file", "first\ntwo\n"),
        ("tee-file", "tea\n"),
        ("sorted", "apple\npear\n"),
        ("unique", "same\n"),
    ] {
        let file = files.iter().find(|file| file.path.ends_with(name)).unwrap();
        assert_eq!(file.edits.len(), 1, "{name}");
        assert_eq!(
            fs::read_to_string(file.edits[0].modified.as_ref().unwrap()).unwrap(),
            expected
        );
    }
    let sorted = files
        .iter()
        .find(|file| file.path.ends_with("sorted"))
        .unwrap();
    assert_eq!(
        fs::read_to_string(sorted.edits[0].original.as_ref().unwrap()).unwrap(),
        "pear\napple\n"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirected_external_output_is_forwarded_through_the_recorder() {
    let root = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    run(root.path(), tracking.clone(), "/bin/sh -c 'printf child; printf error >&2' > out 2> err; cat out > observed; /bin/sh -c 'printf first; printf second >&2; printf third' > combined 2>&1").await;
    let files = tracking.report().unwrap().files;
    assert_eq!(files.len(), 4);
    for (name, expected) in [
        ("out", "child"),
        ("err", "error"),
        ("observed", "child"),
        ("combined", "firstsecondthird"),
    ] {
        let file = files.iter().find(|file| file.path.ends_with(name)).unwrap();
        assert_eq!(
            fs::read_to_string(file.edits[0].modified.as_ref().unwrap()).unwrap(),
            expected
        );
    }
}

/// Jobs that take turns writing one file each keep their own edits: another
/// job's write does not change an after side a job captured, and a job's
/// next edit starts from what the other job left.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_job_cannot_change_an_already_captured_after_side() {
    let root = tempfile::tempdir().unwrap();
    let a = recorder(root.path(), "a");
    let b = recorder(root.path(), "b");
    fs::write(root.path().join("file"), "before\n").unwrap();
    run(root.path(), a.clone(), "echo A > file").await;
    run(root.path(), b.clone(), "echo B > file").await;
    run(root.path(), a.clone(), "echo C > file").await;
    let report = a.report().unwrap();
    let edits: Vec<_> = report.files[0]
        .edits
        .iter()
        .map(|edit| {
            (
                fs::read_to_string(edit.original.as_ref().unwrap()).unwrap(),
                fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("before\n".to_owned(), "A\n".to_owned()),
            ("B\n".to_owned(), "C\n".to_owned())
        ]
    );
}
