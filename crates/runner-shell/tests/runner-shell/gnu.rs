//! A job's standard utilities and its shell's own builtins behave as GNU's
//! and bash's do (`runner.md` § Standard utilities): their messages, exit
//! statuses and options, the paths they take, the copies `cp` makes, and the
//! programs a utility starts.

use demi_runner_shell::testing::{Scope, ShellOptions, execute};
use std::{fs, path::Path};

/// Runs `script` as a job in `root`, with the system's programs on `PATH`,
/// and returns its status, standard output and standard error.
async fn job(root: &Path, script: &str) -> (u8, String, String) {
    let output = tempfile::NamedTempFile::new().unwrap();
    let error = tempfile::NamedTempFile::new().unwrap();
    let mut env = super::home(root);
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    let result = execute(
        script,
        ShellOptions {
            scope: Scope::new(tokio_util::sync::CancellationToken::new(), None),
            login: false,
            cwd: root.into(),
            env,
            stdin: tempfile::tempfile().unwrap(),
            stdout: output.reopen().unwrap(),
            stderr: error.reopen().unwrap(),
        },
    )
    .await
    .unwrap();
    (
        result.code,
        fs::read_to_string(output.path()).unwrap(),
        fs::read_to_string(error.path()).unwrap(),
    )
}

/// A directory with two one-line files, `a` and `b`, a file `afile` that is
/// not executable, and a directory `dir` with a file in it.
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "one\n").unwrap();
    fs::write(root.path().join("b"), "two\n").unwrap();
    fs::write(root.path().join("afile"), "text\n").unwrap();
    fs::create_dir(root.path().join("dir")).unwrap();
    fs::write(root.path().join("dir/inside"), "inside\n").unwrap();
    root
}

/// Each script's status, standard output and standard error are GNU's
/// coreutils, GNU sed 4.9's and bash's, in English. One job runs several
/// utilities one after another, each with its own messages. Runs in about
/// 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn utilities_and_builtins_print_gnu_and_bash_messages() {
    let cases: &[(&str, u8, &str, &str)] = &[
        (
            "ls nope",
            2,
            "",
            "ls: cannot access 'nope': No such file or directory\n",
        ),
        (
            "rm nope",
            1,
            "",
            "rm: cannot remove 'nope': No such file or directory\n",
        ),
        (
            "chmod 600 nope",
            1,
            "",
            "chmod: cannot access 'nope': No such file or directory\n",
        ),
        (
            "cp nope x",
            1,
            "",
            "cp: cannot stat 'nope': No such file or directory\n",
        ),
        (
            "dirname",
            1,
            "",
            "dirname: missing operand\nTry 'dirname --help' for more information.\n",
        ),
        (
            "dirname; ls nope; rm nope; dirname a/b",
            0,
            "a\n",
            concat!(
                "dirname: missing operand\n",
                "Try 'dirname --help' for more information.\n",
                "ls: cannot access 'nope': No such file or directory\n",
                "rm: cannot remove 'nope': No such file or directory\n",
            ),
        ),
        ("wc -l a b | tail -n 1", 0, "2 total\n", ""),
        ("ls -l dir | head -n 1 | cut -d ' ' -f 1", 0, "total\n", ""),
        (
            "ls --help | grep -c 'List directory contents'",
            0,
            "1\n",
            "",
        ),
        (
            "sed -i '' 's/a/b/' a; echo $?; cat a",
            0,
            "2\none\n",
            "sed: can't read s/a/b/: No such file or directory\n",
        ),
        (
            "sed p nope a",
            2,
            "one\none\n",
            "sed: can't read nope: No such file or directory\n",
        ),
        (
            "sed k a",
            1,
            "",
            "sed: -e expression #1, char 1: unknown command: `k'\n",
        ),
        (
            "sed -e p -e k a",
            1,
            "",
            "sed: -e expression #2, char 1: unknown command: `k'\n",
        ),
        (
            "sed s/a/b a",
            1,
            "",
            "sed: -e expression #1, char 5: unterminated `s' command\n",
        ),
        ("sed -i s/a/b/", 4, "", "sed: no input files\n"),
        (
            "sed 2>&1 | head -n 1; sed 2>/dev/null; echo $?",
            0,
            "Usage: sed [OPTION]... [script] [file]...\n1\n",
            "",
        ),
        (
            "cd ./browse",
            1,
            "",
            "cd: ./browse: No such file or directory\n",
        ),
        ("cd afile", 1, "", "cd: afile: Not a directory\n"),
        ("cd a b", 2, "", "cd: too many arguments\n"),
        ("nosuch", 127, "", "nosuch: command not found\n"),
        (
            "./nosuch",
            127,
            "",
            "./nosuch: No such file or directory\n",
        ),
        ("./afile", 126, "", "./afile: Permission denied\n"),
        (
            "source nope.sh",
            1,
            "",
            "nope.sh: No such file or directory\n",
        ),
        ("popd", 1, "", "popd: directory stack empty\n"),
        ("kill 999999", 1, "", "kill: (999999) - No such process\n"),
        ("kill -- -999999", 1, "", "kill: (-999999) - No such process\n"),
        ("kill %9", 1, "", "kill: %9: no such job\n"),
        (
            "kill",
            2,
            "",
            "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n",
        ),
        ("kill abc", 1, "", "kill: `abc': not a pid or valid job spec\n"),
        ("kill -s FOO 1", 1, "", "kill: FOO: invalid signal specification\n"),
        (
            "wait 999999",
            127,
            "",
            "wait: pid 999999 is not a child of this shell\n",
        ),
        ("wait %9", 127, "", "wait: %9: no such job\n"),
        ("jobs -p %9", 1, "", "jobs: %9: no such job\n"),
    ];
    let mut failures = Vec::new();
    for &(script, code, stdout, stderr) in cases {
        let root = fixture();
        let actual = job(root.path(), script).await;
        if (actual.0, actual.1.as_str(), actual.2.as_str()) != (code, stdout, stderr) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `find -exec`, `xargs` and `env` run a bare utility name as the job's own
/// utility, as the shell does, and a path as that program: `--version` is
/// GNU's option, which the system's BSD `sed` on macOS lacks, and on Linux
/// the system's GNU sed names itself differently. Runs in about 0.3 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn programs_a_utility_starts_are_the_jobs_utilities_by_name() {
    let root = fixture();
    let (code, builtin, error) = job(root.path(), "sed --version").await;
    assert_eq!(code, 0, "{error}");
    assert!(!builtin.is_empty());
    for script in [
        "find . -name a -exec sed --version \\;",
        "find . -name a -exec sed --version {} +",
        "echo a | xargs sed --version",
        "env sed --version",
        "env -i sed --version",
    ] {
        let (code, output, error) = job(root.path(), script).await;
        assert_eq!((code, output.as_str(), error.as_str()), (0, builtin.as_str(), ""), "{script}");
    }
    for script in [
        "env /usr/bin/sed --version",
        "find . -name a -exec /usr/bin/sed --version \\;",
    ] {
        let (_, output, _) = job(root.path(), script).await;
        assert_ne!(output, builtin, "{script}");
    }
    // The utility a program starts reads and writes the streams it was
    // given: `xargs` gives its command no input, and `env` passes the job's.
    let (code, output, error) = job(root.path(), "printf 'x\\n' | env sed s/x/y/; echo a | xargs cat").await;
    assert_eq!((code, output.as_str(), error.as_str()), (0, "y\none\n", ""));
}

/// A copy gets the time it was made, unless `-p` keeps the source's, and an
/// existing destination is written in place, so a hard link to it sees the
/// copy; `-c` is a clone where the file system can, as `--reflink=auto`.
/// The destinations are relative to the job's directory, which is not the
/// test's. Runs in about 5 ms.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cp_makes_copies_as_gnu_cp_does() {
    use std::os::unix::fs::MetadataExt;
    let root = fixture();
    let path = |name: &str| root.path().join(name);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
    fs::write(path("src"), "source\n").unwrap();
    fs::File::options()
        .write(true)
        .open(path("src"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    fs::write(path("existing"), "before\n").unwrap();
    fs::hard_link(path("existing"), path("link")).unwrap();
    let inode = fs::metadata(path("existing")).unwrap().ino();
    let (code, output, error) = job(
        root.path(),
        "cp src copy && cp -p src dir/kept && cp src existing && cp -c src cloned && cp --reflink=never -c src clone2",
    )
    .await;
    assert_eq!(code, 0, "{output}{error}");
    let modified = |name: &str| fs::metadata(path(name)).unwrap().modified().unwrap();
    let age = |name: &str| std::time::SystemTime::now().duration_since(modified(name)).unwrap_or_default();
    assert!(age("copy") < std::time::Duration::from_secs(600), "{:?}", age("copy"));
    assert!(age("cloned") < std::time::Duration::from_secs(600));
    let kept = modified("dir/kept").duration_since(old).unwrap_or_else(|error| error.duration());
    assert!(kept < std::time::Duration::from_secs(1), "{kept:?}");
    assert_eq!(fs::metadata(path("existing")).unwrap().ino(), inode);
    assert_eq!(fs::read_to_string(path("link")).unwrap(), "source\n");
    for name in ["copy", "dir/kept", "cloned", "clone2"] {
        assert_eq!(fs::read_to_string(path(name)).unwrap(), "source\n", "{name}");
    }
}

/// A copy carries none of the source's extended attributes unless it
/// preserves them, as GNU cp's does, also where macOS's clonefile(2) copied
/// them with the data. Runs in about 50 ms: macOS's own `xattr` reads them.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cp_copies_extended_attributes_only_when_preserving_them() {
    let root = fixture();
    fs::write(root.path().join("src"), "source\n").unwrap();
    let (code, output, error) = job(
        root.path(),
        "/usr/bin/xattr -w demi.test kept src && cp src copy && cp -c src cloned && cp -p src stamped \
         && cp --preserve=xattr src preserved && cp -a src archived \
         && for f in copy cloned stamped preserved archived; do echo \"$f: $(/usr/bin/xattr $f)\"; done",
    )
    .await;
    assert_eq!((code, error.as_str()), (0, ""), "{output}");
    assert_eq!(
        output,
        "copy: \ncloned: \nstamped: \npreserved: demi.test\narchived: demi.test\n"
    );
}

/// `chmod`, `chown` and `stat -f` take a relative path against the job's
/// directory, and `chown` words a failure as GNU's does. Runs in about
/// 5 ms.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_and_file_system_utilities_take_paths_in_the_jobs_directory() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = fixture();
    let user = fs::metadata(root.path().join("a")).unwrap().uid();
    let (code, output, error) = job(
        root.path(),
        &format!("chmod 600 a && chown {user} a && chown -R {user} dir && stat -f a >/dev/null"),
    )
    .await;
    assert_eq!((code, error.as_str()), (0, ""), "{output}");
    assert_eq!(
        fs::metadata(root.path().join("a")).unwrap().permissions().mode() & 0o777,
        0o600
    );
    if user != 0 {
        let (code, _, error) = job(root.path(), "chown 0 a").await;
        assert_eq!(
            (code, error.as_str()),
            (1, "chown: changing ownership of 'a': Operation not permitted\n")
        );
    }
}

/// Two directory trees, `a` and `b`, that differ in a file at the top, in a
/// file two levels down, in entries on one side only, and in an entry that
/// is a directory on one side and a file on the other.
fn trees() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let files = [
        ("a/x", "one\ntwo\n"),
        ("b/x", "one\nTWO\n"),
        ("a/same", "same\n"),
        ("b/same", "same\n"),
        ("a/sub/deep/f", "A\n"),
        ("b/sub/deep/f", "B\n"),
        ("a/sub/onlya", "only\n"),
        ("b/onlyb", "only\n"),
        ("a/onlydir/in", "in\n"),
        ("a/s/e", "e\n"),
        ("b/s/e", "e\n"),
        ("b/fd", "f\n"),
    ];
    for (path, text) in files {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fs::create_dir(root.path().join("a/fd")).unwrap();
    root
}

/// `diff` compares directories as GNU diff 3.12 does in the C locale, with
/// short options clustered as its getopt reads them: `-r` descends, `-q`
/// only names the files that differ, `-N` compares a file on one side only
/// with an empty one, and without `-r` only the top level is compared. The
/// expected outputs are GNU's for the same trees. Runs in about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_compares_directories_as_gnu_diff_does() {
    let cases: &[(&str, u8, &str, &str)] = &[
        (
            "diff -rq a b",
            1,
            concat!(
                "File a/fd is a directory while file b/fd is a regular file\n",
                "Only in b: onlyb\n",
                "Only in a: onlydir\n",
                "Files a/sub/deep/f and b/sub/deep/f differ\n",
                "Only in a/sub: onlya\n",
                "Files a/x and b/x differ\n",
            ),
            "",
        ),
        (
            "diff -r a b",
            1,
            concat!(
                "File a/fd is a directory while file b/fd is a regular file\n",
                "Only in b: onlyb\n",
                "Only in a: onlydir\n",
                "diff -r a/sub/deep/f b/sub/deep/f\n",
                "1c1\n< A\n---\n> B\n",
                "Only in a/sub: onlya\n",
                "diff -r a/x b/x\n",
                "2c2\n< two\n---\n> TWO\n",
            ),
            "",
        ),
        (
            "diff a b",
            1,
            concat!(
                "File a/fd is a directory while file b/fd is a regular file\n",
                "Only in b: onlyb\n",
                "Only in a: onlydir\n",
                "Common subdirectories: a/s and b/s\n",
                "Common subdirectories: a/sub and b/sub\n",
                "diff a/x b/x\n",
                "2c2\n< two\n---\n> TWO\n",
            ),
            "",
        ),
        (
            "diff -Naur a b | cut -f 1",
            0,
            concat!(
                "File a/fd is a directory while file b/fd is a regular file\n",
                "diff -Naur a/onlyb b/onlyb\n",
                "--- a/onlyb\n+++ b/onlyb\n@@ -0,0 +1 @@\n+only\n",
                "diff -Naur a/onlydir/in b/onlydir/in\n",
                "--- a/onlydir/in\n+++ b/onlydir/in\n@@ -1 +0,0 @@\n-in\n",
                "diff -Naur a/sub/deep/f b/sub/deep/f\n",
                "--- a/sub/deep/f\n+++ b/sub/deep/f\n@@ -1 +1 @@\n-A\n+B\n",
                "diff -Naur a/sub/onlya b/sub/onlya\n",
                "--- a/sub/onlya\n+++ b/sub/onlya\n@@ -1 +0,0 @@\n-only\n",
                "diff -Naur a/x b/x\n",
                "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
            ),
            "",
        ),
        ("diff -rqN a b >/dev/null", 1, "", ""),
        (
            "seq 1 9 > p; seq 1 9 | sed s/5/X/ > q; diff -1ru p q | tail -n +3",
            0,
            "@@ -4,3 +4,3 @@\n 4\n-5\n+X\n 6\n",
            "",
        ),
        ("diff -r a/s b/s", 0, "", ""),
        ("diff -rs a/s b/s", 0, "Files a/s/e and b/s/e are identical\n", ""),
        (
            "diff -q a/sub b/sub",
            1,
            "Common subdirectories: a/sub/deep and b/sub/deep\nOnly in a/sub: onlya\n",
            "",
        ),
        ("diff -N b/onlyb nope", 1, "1d0\n< only\n", ""),
        ("diff a/x b", 1, "2c2\n< two\n---\n> TWO\n", ""),
        (
            "diff -r -- a/sub b/sub",
            1,
            "diff -r -- a/sub/deep/f b/sub/deep/f\n1c1\n< A\n---\n> B\nOnly in a/sub: onlya\n",
            "",
        ),
        ("diff -rq a nope", 2, "", "diff: nope: No such file or directory\n"),
        ("diff -N nope b/onlyb", 1, "0a1\n> only\n", ""),
        ("diff -N nope other", 2, "", "diff: nope: No such file or directory\ndiff: other: No such file or directory\n"),
    ];
    let mut failures = Vec::new();
    for &(script, code, stdout, stderr) in cases {
        let root = trees();
        let actual = job(root.path(), script).await;
        if (actual.0, actual.1.as_str(), actual.2.as_str()) != (code, stdout, stderr) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    // An absent file's time in a unified header is the epoch, in the local
    // time zone.
    let root = trees();
    let (code, output, error) = job(root.path(), "diff -Nu a/sub/onlya b/sub/onlya").await;
    assert_eq!((code, error.as_str()), (1, ""), "{output}");
    let absent = output.lines().nth(1).unwrap();
    assert!(
        absent.starts_with("+++ b/sub/onlya\t1970-01-01 ") || absent.starts_with("+++ b/sub/onlya\t1969-12-31 "),
        "{absent}"
    );
}

/// Runs each case's script in a new directory that the shell script `setup`
/// prepared, and fails naming every case whose status, standard output or
/// standard error differs from the expected.
async fn expect_cases(setup: &str, cases: &[(&str, u8, &str, &str)]) {
    let mut failures = Vec::new();
    for &(script, code, stdout, stderr) in cases {
        let root = tempfile::tempdir().unwrap();
        let (status, _, error) = job(root.path(), setup).await;
        assert_eq!((status, error.as_str()), (0, ""), "{setup}");
        let actual = job(root.path(), script).await;
        if (actual.0, actual.1.as_str(), actual.2.as_str()) != (code, stdout, stderr) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A utility whose reader has closed the pipe ends with 141 and nothing on
/// stderr, as SIGPIPE ends GNU's in a shell: `diff` panicked, `cmp` and the
/// coreutils printed "Broken pipe" and exited 1 or 2, and `du`'s walk
/// reported its printer's end as "sending on a closed channel". Each
/// output is well over a 64 KiB pipe buffer, so the reader is gone before
/// the writer ends. Runs in about 1 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_pipe_ends_utilities_quietly_as_sigpipe_does() {
    let setup = "seq -f '%060g' 1 2000 > l1; seq -f '%060g' 2001 4000 > l2; \
                 mkdir t; seq -f 't/%060g' 1 3000 | xargs touch";
    let cases: Vec<(String, u8, &str, &str)> = [
        "diff l1 l2", "diff -u l1 l2", "diff -y l1 l2", "cmp -l l1 l2", "cat l1 l1 l1", "sort l1", "du -a t",
    ]
    .into_iter()
    .map(|producer| (format!("{producer} | head -n 1 >/dev/null; echo ${{PIPESTATUS[0]}}"), 0, "141\n", ""))
    .collect();
    let cases: Vec<(&str, u8, &str, &str)> =
        cases.iter().map(|(script, code, out, err)| (script.as_str(), *code, *out, *err)).collect();
    expect_cases(setup, &cases).await;
}

/// `diff` reports files with a NUL in their first 4 KiB block as GNU's does,
/// as binary files that differ, unless `-a` compares them as text. The
/// expected outputs are GNU diff 3.12's. Runs in about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_reports_binary_files_as_gnu_diff_does() {
    let setup = "printf 'a\\0b\\n' > z1; printf 'a\\0c\\n' > z2; cp z1 z3; printf 'x\\n' > tx; \
                 mkdir d1 d2; cp z1 d1/z; cp z2 d2/z; \
                 head -c 4000 /dev/zero | tr '\\0' a > b4; printf '\\0' >> b4; \
                 head -c 5000 /dev/zero | tr '\\0' a > b5; printf '\\0\\n' >> b5; printf 'x\\n' > c";
    expect_cases(
        setup,
        &[
        ("diff z1 z2", 1, "Binary files z1 and z2 differ\n", ""),
        ("diff -q z1 z2", 1, "Files z1 and z2 differ\n", ""),
        ("diff -a z1 z2", 1, "1c1\n< a\u{0}b\n---\n> a\u{0}c\n", ""),
        ("diff --text z1 z2 | wc -c", 0, "20\n", ""),
        ("diff z1 z3", 0, "", ""),
        ("diff tx z1", 1, "Binary files tx and z1 differ\n", ""),
        ("diff -r d1 d2", 1, "Binary files d1/z and d2/z differ\n", ""),
        ("diff b4 c", 1, "Binary files b4 and c differ\n", ""),
        ("diff b5 c | head -c 6", 0, "1c1\n< ", ""),
        ],
    )
    .await;
}

/// `-w`, `-b`, `-i` and `-B` compare lines as GNU diff's do: what they
/// ignore makes no difference, the status included, and the printed lines
/// are each file's own. The expected outputs are GNU diff 3.12's. Runs in
/// about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_ignores_case_and_white_space_as_gnu_diff_does() {
    let setup = "printf 'a b\\nfoo  bar\\nX\\n\\nend\\n' > p; printf 'a  b\\nfoo bar \\nx\\nend\\n' > q; \
                 printf 'a\\n\\n\\nb\\nc\\n' > r; printf 'a\\nb\\nC\\n' > s; \
                 printf 'a\\n  \\nb\\n' > t1; printf 'a\\nb\\n' > t2; printf 'a\\n' > n1; printf 'a' > n2";
    expect_cases(
        setup,
        &[
        ("diff -w p q", 1, "3,4c3\n< X\n< \n---\n> x\n", ""),
        ("diff -b p q", 1, "3,4c3\n< X\n< \n---\n> x\n", ""),
        ("diff -i p q", 1, "1,2c1,2\n< a b\n< foo  bar\n---\n> a  b\n> foo bar \n4d3\n< \n", ""),
        ("diff -B p q", 1, "1,4c1,3\n< a b\n< foo  bar\n< X\n< \n---\n> a  b\n> foo bar \n> x\n", ""),
        ("diff -wi p q", 1, "4d3\n< \n", ""),
        ("diff -wiB p q", 0, "", ""),
        ("diff -s -wiB p q", 0, "Files p and q are identical\n", ""),
        ("diff -q -w p q", 1, "Files p and q differ\n", ""),
        ("diff -c -b p q | tail -n +3", 0, "***************\n*** 1,5 ****\n  a b\n  foo  bar\n! X\n! \n  end\n--- 1,4 ----\n  a  b\n  foo bar \n! x\n  end\n", ""),
        ("diff -Bu r s | tail -n +3", 0, "@@ -1,5 +1,3 @@\n a\n-\n-\n b\n-c\n+C\n", ""),
        ("diff -B t1 t2", 1, "2d1\n<   \n", ""),
        ("diff -w n1 n2", 0, "", ""),
        ("diff --ignore-all-space --ignore-case p q", 1, "4d3\n< \n", ""),
        ],
    )
    .await;
}

/// `-x` and `-X` leave out of a directory comparison the names their
/// patterns match, but never an operand. The expected outputs are GNU diff
/// 3.12's. Runs in about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_excludes_names_as_gnu_diff_does() {
    let setup = "mkdir -p d1/node_modules/x d2 d1/sub d2/sub; echo 1 > d1/f.o; echo 2 > d2/f.o; \
                 echo 1 > d1/k; echo 2 > d2/k; echo q > d1/node_modules/x/y; echo 1 > d1/sub/f.o; \
                 echo 1 > d1/.hid; printf '*.o\\nnode_modules\\n\\n' > ex";
    expect_cases(
        setup,
        &[
        ("diff -rq -x '*.o' d1 d2", 1, "Only in d1: .hid\nFiles d1/k and d2/k differ\nOnly in d1: node_modules\n", ""),
        ("diff -rq -X ex d1 d2", 1, "Only in d1: .hid\nFiles d1/k and d2/k differ\n", ""),
        ("diff -rq --exclude=k --exclude '*.o' d1 d2", 1, "Only in d1: .hid\nOnly in d1: node_modules\n", ""),
        ("diff -r -x k -x '*.o' d1 d2", 1, "Only in d1: .hid\nOnly in d1: node_modules\n", ""),
        ("diff -rq -x '*' d1 d2", 0, "", ""),
        ("diff -rq -x d1 d1 d2", 1, "Only in d1: .hid\nFiles d1/f.o and d2/f.o differ\nFiles d1/k and d2/k differ\nOnly in d1: node_modules\nOnly in d1/sub: f.o\n", ""),
        ("diff -rqx k -x '*.o' d1 d2", 1, "Only in d1: .hid\nOnly in d1: node_modules\n", ""),
        ("diff -rq --exclude-from=ex d1 d2", 1, "Only in d1: .hid\nFiles d1/k and d2/k differ\n", ""),
        ("diff -rq -X nope d1 d2", 2, "", "diff: nope: No such file or directory\n"),
        ],
    )
    .await;
}

/// `diff` words a usage error as GNU's getopt does, with exit status 2, and
/// takes an unambiguous abbreviation of a long option. The expected outputs
/// are GNU diff 3.12's. Runs in about 0.02 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_usage_errors_are_gnu_diffs() {
    let setup = "echo a > a; echo b > b; mkdir d1 d2; echo 1 > d1/f; echo 2 > d2/f";
    expect_cases(
        setup,
        &[
        ("diff -k a b", 2, "", "diff: invalid option -- 'k'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff -rqk a b", 2, "", "diff: invalid option -- 'k'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --foo a b", 2, "", "diff: unrecognized option '--foo'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --no-such=3 a b", 2, "", "diff: unrecognized option '--no-such=3'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --ign a b", 2, "", "diff: option '--ign' is ambiguous; possibilities: '--ignore-all-space' '--ignore-blank-lines' '--ignore-case' '--ignore-file-name-case' '--ignore-matching-lines' '--ignore-space-change' '--ignore-tab-expansion' '--ignore-trailing-space'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --exclu=k a b", 2, "", "diff: option '--exclu=k' is ambiguous; possibilities: '--exclude' '--exclude-from'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff -x", 2, "", "diff: option requires an argument -- 'x'\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --exclude-f", 2, "", "diff: option '--exclude-from' requires an argument\ndiff: Try 'diff --help' for more information.\n"),
        ("diff --recur --brie d1 d2", 1, "Files d1/f and d2/f differ\n", ""),
        ("diff --ignore-cas a b", 1, "1c1\n< a\n---\n> b\n", ""),
        ],
    )
    .await;
}

/// Hunks are GNU's: changes at most twice the context apart share a hunk, a
/// context hunk marks a group that deletes and inserts as changes, and an
/// ed script runs last change first. The expected outputs are GNU diff
/// 3.12's. Runs in about 0.02 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_hunks_are_gnu_diffs() {
    let setup = "printf '1\\n2\\n3\\n4\\n5\\n6\\n' > m1; printf '1\\nX\\n3\\n4\\n6\\n' > m2; \
                 printf 'a\\nb\\nc\\nd\\n' > g1; printf 'a\\nB\\nd\\n' > g2; seq 1 20 > s1; \
                 seq 1 20 | sed -e s/^5$/X/ -e s/^12$/Y/ -e s/^19$/Z/ > s2";
    expect_cases(
        setup,
        &[
        ("diff -e m1 m2", 1, "5d\n2c\nX\n.\n", ""),
        ("diff -e g1 g2", 1, "2,3c\nB\n.\n", ""),
        ("diff -c g1 g2 | tail -n +3", 0, "***************\n*** 1,4 ****\n  a\n! b\n! c\n  d\n--- 1,3 ----\n  a\n! B\n  d\n", ""),
        ("diff -c m1 m2 | tail -n +3", 0, "***************\n*** 1,6 ****\n  1\n! 2\n  3\n  4\n- 5\n  6\n--- 1,5 ----\n  1\n! X\n  3\n  4\n  6\n", ""),
        ("diff -u s1 s2 | tail -n +3", 0, "@@ -2,19 +2,19 @@\n 2\n 3\n 4\n-5\n+X\n 6\n 7\n 8\n 9\n 10\n 11\n-12\n+Y\n 13\n 14\n 15\n 16\n 17\n 18\n-19\n+Z\n 20\n", ""),
        ("diff -U2 s1 s2 | tail -n +3", 0, "@@ -3,5 +3,5 @@\n 3\n 4\n-5\n+X\n 6\n 7\n@@ -10,5 +10,5 @@\n 10\n 11\n-12\n+Y\n 13\n 14\n@@ -17,4 +17,4 @@\n 17\n 18\n-19\n+Z\n 20\n", ""),
        ],
    )
    .await;
}

/// `diff` compares two 300,000-line files that mostly differ in bounded
/// time and memory, as GNU's does: it runs inside the runner's process, so
/// a line diff that grows with the product of the files' lengths, as the
/// one before did, would exhaust the memory of the runner and every job on
/// its Host. Nine lines in ten differ. Runs in about 1 s; the budget allows
/// 30 s on a loaded machine.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_compares_large_files_in_bounded_time_and_memory() {
    const LINES: usize = 300_000;
    let root = tempfile::tempdir().unwrap();
    let first: String = (0..LINES).map(|line| format!("line {line}\n")).collect();
    let second: String = (0..LINES)
        .map(|line| if line % 10 == 0 { format!("line {line}\n") } else { format!("other {line}\n") })
        .collect();
    fs::write(root.path().join("first"), first).unwrap();
    fs::write(root.path().join("second"), second).unwrap();
    let started = std::time::Instant::now();
    let (code, output, error) = job(root.path(), "diff first second | wc -l; diff first second >/dev/null").await;
    let elapsed = started.elapsed();
    assert_eq!((code, error.as_str()), (1, ""), "{output}");
    let lines: usize = output.lines().next().unwrap().trim().parse().unwrap();
    assert!(lines > LINES, "{output}");
    assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
}
