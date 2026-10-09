use demi_command_protocol::{EditContext, EditKind, PathChange};
use demi_command_sdk::edits::Recorder;
use demi_runner_shell::testing::{Scope, ShellOptions, execute};
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
                // `mktemp` makes its file in the temporary directory the
                // job's environment names, which goes with the test's.
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

/// The edit `mv` makes: a file renamed over another, as an editor or a
/// formatter replaces it from a temporary file, is that file modified with
/// both sides, as is `sed -i`'s rename; a backup it keeps is no edit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_replaced_by_a_rename_is_modified_with_both_sides() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    for (name, contents) in [
        ("page.ts", "old page\n"),
        ("style.css", "old style\n"),
        ("config", "debug=1\n"),
    ] {
        fs::write(root.path().join(name), contents).unwrap();
    }
    // A formatter's output, written before the job.
    fs::write(outside.path().join("style.css"), "new style\n").unwrap();
    let outside = outside.path().display();
    run(
        root.path(),
        tracking.clone(),
        &format!(
            "printf 'new page\\n' > {outside}/page.ts; mv {outside}/page.ts page.ts; \
             mv -b {outside}/style.css style.css; sed -i 's/1/2/' config"
        ),
    )
    .await;
    let edits: Vec<_> = tracking
        .report()
        .unwrap()
        .files
        .iter()
        .map(|file| {
            assert_eq!(file.kind, EditKind::Modified, "{}", file.path);
            assert_eq!(file.edits.len(), 1, "{}", file.path);
            let edit = &file.edits[0];
            (
                Path::new(&file.path).strip_prefix(root.path()).unwrap().to_owned(),
                fs::read_to_string(edit.original.as_ref().unwrap()).unwrap(),
                fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("page.ts".into(), "old page\n".into(), "new page\n".into()),
            ("style.css".into(), "old style\n".into(), "new style\n".into()),
            ("config".into(), "debug=1\n".into(), "debug=2\n".into()),
        ] as [(std::path::PathBuf, String, String); 3]
    );
}

/// `sed -i` with BSD's separate suffix, as models on macOS write it, edits
/// the file and records the edit: `-i ''` keeps no backup, `-i .bak` keeps
/// one, which is no edit. A dotfile after `-i` with the script already
/// given by `-e` is the file to edit, not a suffix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sed_in_place_with_a_separate_bsd_suffix_records_its_edit() {
    let root = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    fs::write(root.path().join("config"), "level=1\n").unwrap();
    fs::write(root.path().join("f"), "a\n").unwrap();
    fs::write(root.path().join(".env"), "x\n").unwrap();
    fs::write(root.path().join("g"), "c\n").unwrap();
    run(
        root.path(),
        tracking.clone(),
        "sed -i '' 's/level=1/level=2/' config; sed -i .bak 's/a/b/' f; \
         sed -e 's/x/y/' -i .env; sed -e 's/c/d/' -i .bak g",
    )
    .await;
    let mut names: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !["job", "edits.lock"].contains(&name.as_str()))
        .collect();
    names.sort();
    assert_eq!(names, [".env", "config", "f", "f.bak", "g", "g.bak"]);
    assert_eq!(fs::read_to_string(root.path().join("f.bak")).unwrap(), "a\n");
    assert_eq!(fs::read_to_string(root.path().join("g.bak")).unwrap(), "c\n");
    let edits: Vec<_> = tracking
        .report()
        .unwrap()
        .files
        .iter()
        .map(|file| {
            assert_eq!(file.kind, EditKind::Modified, "{}", file.path);
            assert_eq!(file.edits.len(), 1, "{}", file.path);
            let edit = &file.edits[0];
            (
                Path::new(&file.path).file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read_to_string(edit.original.as_ref().unwrap()).unwrap(),
                fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("config".to_owned(), "level=1\n".to_owned(), "level=2\n".to_owned()),
            ("f".to_owned(), "a\n".to_owned(), "b\n".to_owned()),
            (".env".to_owned(), "x\n".to_owned(), "y\n".to_owned()),
            ("g".to_owned(), "c\n".to_owned(), "d\n".to_owned()),
        ]
    );
}

/// A move carries the job's edits to the new name: a file written outside
/// the workspace and moved in is added with its contents, an edited file
/// renamed shows its edit under the new name, as do the files of a renamed
/// folder, and a file moved unchanged records nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_carries_the_jobs_edits_to_the_new_name() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    fs::write(root.path().join("a.ts"), "a\n").unwrap();
    fs::write(root.path().join("unchanged.ts"), "same\n").unwrap();
    fs::create_dir(root.path().join("old")).unwrap();
    fs::write(root.path().join("old/x.ts"), "x\n").unwrap();
    let outside = outside.path().display();
    run(
        root.path(),
        tracking.clone(),
        &format!(
            "printf 'created\\n' > {outside}/new.ts; mv {outside}/new.ts new.ts; \
             printf 'more\\n' >> a.ts; mv a.ts b.ts; \
             mv unchanged.ts still.ts; \
             printf 'y\\n' >> old/x.ts; mv old moved"
        ),
    )
    .await;
    let edits: Vec<_> = tracking
        .report()
        .unwrap()
        .files
        .iter()
        .map(|file| {
            assert_eq!(file.kind, EditKind::Added, "{}", file.path);
            assert_eq!(file.edits.len(), 1, "{}", file.path);
            let edit = &file.edits[0];
            (
                Path::new(&file.path).strip_prefix(root.path()).unwrap().to_owned(),
                edit.original.as_ref().map(|path| fs::read_to_string(path).unwrap()),
                fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("new.ts".into(), None, "created\n".into()),
            ("b.ts".into(), Some("a\n".into()), "a\nmore\n".into()),
            ("moved/x.ts".into(), Some("x\n".into()), "x\ny\n".into()),
        ] as [(std::path::PathBuf, Option<String>, String); 3]
    );
}

/// A job lists its renames and removals, in order, beside its edits, so a
/// request can follow its earlier calls' files (`edit-tracking.md`
/// § A request): a move of a file the job did not change, a replacement
/// from outside the workspace, and a removed folder named once by its own
/// path; an `rm` that removed nothing lists nothing. A file removed and
/// made again starts anew.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_lists_its_renames_and_removals_in_order() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tracking = recorder(root.path(), "job");
    fs::write(root.path().join("untouched.ts"), "same\n").unwrap();
    fs::write(root.path().join("page.ts"), "old page\n").unwrap();
    fs::create_dir_all(root.path().join("scratch/deep")).unwrap();
    fs::write(root.path().join("scratch/a.txt"), "a\n").unwrap();
    fs::write(root.path().join("scratch/deep/b.txt"), "b\n").unwrap();
    let outside_path = outside.path().display();
    run(
        root.path(),
        tracking.clone(),
        &format!(
            "mv untouched.ts renamed.ts; \
             printf 'new page\\n' > {outside_path}/page.ts.new; mv {outside_path}/page.ts.new page.ts; \
             rm -r scratch/; rm -f missing.txt; \
             printf 'first\\n' > again.txt; rm again.txt; printf 'second\\n' > again.txt"
        ),
    )
    .await;
    let report = tracking.report().unwrap();
    let at = |name: &str| root.path().join(name).to_string_lossy().into_owned();
    assert_eq!(
        report.path_changes,
        [
            PathChange::Renamed {
                from: at("untouched.ts"),
                to: at("renamed.ts"),
            },
            PathChange::Renamed {
                from: outside.path().join("page.ts.new").to_string_lossy().into_owned(),
                to: at("page.ts"),
            },
            PathChange::Removed {
                path: at("scratch"),
            },
            PathChange::Removed {
                path: at("again.txt"),
            },
        ]
    );
    let files: Vec<_> = report
        .files
        .iter()
        .map(|file| {
            let sides: Vec<_> = file
                .edits
                .iter()
                .map(|edit| {
                    (
                        edit.original.as_ref().map(|path| fs::read_to_string(path).unwrap()),
                        fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
                    )
                })
                .collect();
            (
                Path::new(&file.path).strip_prefix(root.path()).unwrap().to_owned(),
                file.kind,
                sides,
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            (
                "page.ts".into(),
                EditKind::Modified,
                vec![(Some("old page\n".into()), "new page\n".into())]
            ),
            (
                "again.txt".into(),
                EditKind::Added,
                vec![(None, "second\n".into())]
            ),
        ] as [(std::path::PathBuf, EditKind, Vec<(Option<String>, String)>); 2]
    );
}
