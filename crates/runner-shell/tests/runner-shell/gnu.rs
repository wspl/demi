//! A job's standard utilities and its shell's own builtins behave as GNU's
//! and bash's do (`crates/runner-shell/src/utilities.rs`): their messages, exit
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
    // Lossy, so that output that is not UTF-8 fails a comparison legibly.
    let read = |path: &Path| String::from_utf8_lossy(&fs::read(path).unwrap()).into_owned();
    (result.code, read(output.path()), read(error.path()))
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
            "stat",
            1,
            "",
            "stat: missing operand\nTry 'stat --help' for more information.\n",
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
        // Windows runs any file, and one that is not a program fails as
        // bash's does on Linux.
        (
            "./afile",
            126,
            "",
            if cfg!(windows) {
                "./afile: cannot execute binary file: Exec format error\n"
            } else {
                "./afile: Permission denied\n"
            },
        ),
        (
            "source nope.sh",
            1,
            "",
            "nope.sh: No such file or directory\n",
        ),
        ("popd", 1, "", "popd: directory stack empty\n"),
        ("kill 999999", 1, "", "kill: (999999) - No such process\n"),
        #[cfg(unix)]
        ("kill -- -999999", 1, "", "kill: (-999999) - No such process\n"),
        // Windows has no process groups (`crates/runner-shell/src/process_builtins.rs`).
        #[cfg(windows)]
        ("kill -- -999999", 1, "", "kill: (-999999) - Windows has no process groups\n"),
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
        if (actual.0, actual.1.as_str()) != (code, stdout) || !super::same_errors(stderr, &actual.2) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// An error of the operating system that uutils gives no message of its own
/// is worded as the system words it, as GNU's `ls` does, never as Rust's
/// debug form: on Windows `ls /c/Users` printed `unknown io error:
/// '/c/Users', 'Os { code: 123, kind: InvalidFilename, message: "The
/// filename, directory name, or volume label syntax is incorrect." }'`. A
/// name longer than any file system allows gives such an error everywhere.
/// Runs in about 0.01 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ls_words_an_error_without_a_message_of_its_own_as_the_system_does() {
    let root = fixture();
    let name = "a".repeat(300);
    let (code, output, errors) = job(root.path(), &format!("ls {name}")).await;
    assert_eq!((code, output.as_str()), (2, ""));
    let expected = format!("ls: cannot access '{name}': File name too long\n");
    assert!(super::same_errors(&expected, &errors), "{errors}");
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
/// them with the data. Only the attribute the test set is read: macOS 27
/// gives every new file a `com.apple.provenance` of its own, which the copy
/// has whatever cp does. Runs in about 50 ms: macOS's own `xattr` reads it.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cp_copies_extended_attributes_only_when_preserving_them() {
    let root = fixture();
    fs::write(root.path().join("src"), "source\n").unwrap();
    let (code, output, error) = job(
        root.path(),
        "/usr/bin/xattr -w demi.test kept src && cp src copy && cp -c src cloned && cp -p src stamped \
         && cp --preserve=xattr src preserved && cp -a src archived \
         && for f in copy cloned stamped preserved archived; do \
         echo \"$f: $(/usr/bin/xattr -p demi.test $f 2>/dev/null)\"; done",
    )
    .await;
    assert_eq!((code, error.as_str()), (0, ""), "{output}");
    assert_eq!(
        output,
        "copy: \ncloned: \nstamped: \npreserved: kept\narchived: kept\n"
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

/// On Windows, which has no Unix mode or owner, `chmod` and `chown` check
/// what GNU's check and fail as GNU's do, a missing operand, an invalid
/// mode, an unknown user or group or a missing file, while a valid change
/// succeeds and changes nothing; `stat` takes GNU's directives with their
/// flags and widths, `-f` included, and shows the mode `ls -l` shows.
/// Before, `chmod` and `chown` succeeded silently whatever they were given,
/// and `stat` failed on `%04a`, `%A` and `-f`. Runs in about 0.05 s.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_utilities_and_stat_answer_as_gnus_do_on_windows() {
    let user = std::env::var("USERNAME").unwrap();
    let try_help = |name: &str| format!("Try '{name} --help' for more information.\n");
    let cases: Vec<(String, u8, String, String)> = vec![
        ("chmod".into(), 1, "".into(), format!("chmod: missing operand\n{}", try_help("chmod"))),
        ("chmod 644".into(), 1, "".into(), format!("chmod: missing operand after '644'\n{}", try_help("chmod"))),
        ("chmod abc a".into(), 1, "".into(), format!("chmod: invalid mode: 'abc'\n{}", try_help("chmod"))),
        ("chmod 644 nope".into(), 1, "".into(), "chmod: cannot access 'nope': No such file or directory\n".into()),
        (
            "chmod 644 a && chmod +x a && chmod -w a && chmod u=rwx,go= a && chmod -R u+w dir && echo changed".into(),
            0,
            "changed\n".into(),
            "".into(),
        ),
        ("chmod -v 644 a".into(), 0, "mode of 'a' retained as 0777 (rwxrwxrwx)\n".into(), "".into()),
        ("chown".into(), 1, "".into(), format!("chown: missing operand\n{}", try_help("chown"))),
        ("chown nosuchaccount a".into(), 1, "".into(), "chown: invalid user: 'nosuchaccount'\n".into()),
        ("chown 0:nosuchaccount a".into(), 1, "".into(), "chown: invalid group: '0:nosuchaccount'\n".into()),
        ("chown 0 nope".into(), 1, "".into(), "chown: cannot access 'nope': No such file or directory\n".into()),
        (format!("chown {user}: a && chown -R 1000:1000 dir && echo owned"), 0, "owned\n".into(), "".into()),
        ("stat -c '%04a %A %F %s %h' a".into(), 0, "0777 -rwxrwxrwx regular file 4 1\n".into(), "".into()),
        (
            "stat -c '%a|%-5s|%5s|%05s|%F' a dir".into(),
            0,
            "777|4    |    4|00004|regular file\n777|0    |    0|00000|directory\n".into(),
            "".into(),
        ),
        ("stat -f -c '%T %l' a".into(), 0, "ntfs 255\n".into(), "".into()),
        ("stat -c %U a".into(), 0, format!("{user}\n"), "".into()),
        ("stat --printf '%n\\n' a".into(), 0, "a\n".into(), "".into()),
        // GNU's `stat` words it so wherever it reads files with statx(2).
        ("stat nope".into(), 1, "".into(), "stat: cannot statx 'nope': No such file or directory\n".into()),
    ];
    let mut failures = Vec::new();
    for (script, code, stdout, stderr) in &cases {
        let root = fixture();
        let actual = job(root.path(), script).await;
        if (actual.0, actual.1.as_str()) != (*code, stdout.as_str()) || !super::same_errors(stderr, &actual.2) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A utility that writes from a worker thread of its own, as `du` prints
/// from one, writes to the job's output when nothing else in the job has
/// waited on a file yet. Before, `du` alone in a job failed with "there is
/// no reactor running", since that thread is outside the shell's runtime.
/// Runs in about 5 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_utilitys_worker_thread_writes_the_jobs_output() {
    let root = fixture();
    assert_eq!(job(root.path(), "du -b a").await, (0, "4\ta\n".into(), String::new()));
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
        if (actual.0, actual.1.as_str()) != (code, stdout) || !super::same_errors(stderr, &actual.2) {
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
        if (actual.0, actual.1.as_str()) != (code, stdout) || !super::same_errors(stderr, &actual.2) {
            failures.push(format!(
                "{script}\n  expected {code} {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A utility that walks a folder prints each path it builds as GNU's does,
/// on every system: the folder as the user gave it, then the names below it
/// joined with `/`, so on Windows `d\sub` prints its file as `d\sub/g`.
/// Before, `ls -R` headed a subfolder and `cp -rv` named each source by its
/// absolute path on every system; on Windows every walk joined names with
/// `\`, and `du` and `rm -rv` printed absolute paths too. Runs in about
/// 0.1 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn walking_utilities_print_the_paths_they_build_as_gnus_do() {
    let setup = "mkdir -p d/sub && echo x > d/f && echo x > d/sub/g";
    #[allow(unused_mut, reason = "only Windows adds its own cases")]
    let mut cases = vec![
        ("find d | sort", 0, "d\nd/f\nd/sub\nd/sub/g\n", ""),
        ("ls -R d", 0, "d:\nf\nsub\n\nd/sub:\ng\n", ""),
        ("du d | cut -f 2", 0, "d/sub\nd\n", ""),
        ("grep -r x d | sort", 0, "d/f:x\nd/sub/g:x\n", ""),
        ("rg x d | sort", 0, "d/f:x\nd/sub/g:x\n", ""),
        ("rg --files d | sort", 0, "d/f\nd/sub/g\n", ""),
        (
            "cp -rv d e | sort",
            0,
            "'d' -> 'e'\n'd/f' -> 'e/f'\n'd/sub' -> 'e/sub'\n'd/sub/g' -> 'e/sub/g'\n",
            "",
        ),
        (
            "cp -r d e && rm -rv e | sort",
            0,
            "removed 'e/f'\nremoved 'e/sub/g'\nremoved directory 'e'\nremoved directory 'e/sub'\n",
            "",
        ),
        (
            "chmod -Rv u+w d | sed 's/ retained.*//' | sort",
            0,
            "mode of 'd'\nmode of 'd/f'\nmode of 'd/sub'\nmode of 'd/sub/g'\n",
            "",
        ),
    ];
    #[cfg(windows)]
    cases.extend([
        (r"find 'd\sub'", 0, "d\\sub\nd\\sub/g\n", ""),
        (r"grep -r x 'd\sub'", 0, "d\\sub/g:x\n", ""),
        (r"ls -R 'd\.'", 0, "d\\.:\nf\nsub\n\nd\\./sub:\ng\n", ""),
    ]);
    expect_cases(setup, &cases).await;
}

/// A recursive `chmod` or `chown` on Windows that cannot read a child
/// reports it, unless `-f`, goes on with the rest and ends with 1, as GNU's
/// walk does; before, it skipped the child silently and ended with 0. The
/// child is a name ending in a dot, which Windows lists but cannot open by
/// its plain path, so the test makes it through a `\\?\` path. Runs in
/// about 0.05 s.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_permission_walk_on_windows_reports_a_child_it_cannot_read() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("d")).unwrap();
    fs::write(root.path().join("d/f"), "x\n").unwrap();
    let unreadable = format!(r"\\?\{}\d\dot.", root.path().display());
    fs::write(&unreadable, "x\n").unwrap();
    let mut failures = Vec::new();
    for (script, stdout, stderr) in [
        (
            "chmod -Rv 644 d",
            "mode of 'd' retained as 0777 (rwxrwxrwx)\nmode of 'd/f' retained as 0777 (rwxrwxrwx)\n",
            "chmod: cannot access 'd/dot.': No such file or directory\n",
        ),
        ("chmod -Rf 644 d", "", ""),
        ("chown -R 0 d", "", "chown: cannot access 'd/dot.': No such file or directory\n"),
    ] {
        let actual = job(root.path(), script).await;
        if (actual.0, actual.1.as_str()) != (1, stdout) || !super::same_errors(stderr, &actual.2) {
            failures.push(format!(
                "{script}\n  expected 1 {stdout:?} {stderr:?}\n  actual   {} {:?} {:?}",
                actual.0, actual.1, actual.2
            ));
        }
    }
    fs::remove_file(&unreadable).unwrap();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `cp -s` makes each link as GNU's does: to the source as named, which
/// must be absolute unless the link is in the current directory, where a
/// relative one still reaches it; otherwise each file is reported and the
/// copy goes on, ending with 1. Before, `cp -rs d e` linked `e/f` to `d/f`,
/// which from `e` names `e/d/f`, and ended with 0. Runs in about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cp_links_to_its_sources_as_gnus_cp_does() {
    // Windows makes no link without a privilege, so only the refusal,
    // which makes none, runs everywhere.
    #[allow(unused_mut, reason = "only Unix makes links")]
    let mut cases = vec![(
        "cp -rs d e 2> err; s=$?; sort err; find e | sort; echo $s",
        0,
        "cp: e/f: can make relative symbolic links only in current directory\n\
         cp: e/sub/g: can make relative symbolic links only in current directory\n\
         e\ne/sub\n1\n",
        "",
    )];
    #[cfg(unix)]
    cases.extend([
        (
            r#"cp -rs "$PWD/d" e && readlink e/sub/g | sed "s|^$PWD/||" && cat e/sub/g"#,
            0,
            "d/sub/g\nx\n",
            "",
        ),
        ("cp -s d/f link && readlink link && cat link", 0, "d/f\nx\n", ""),
    ]);
    expect_cases("mkdir -p d/sub && echo x > d/f && echo x > d/sub/g", &cases).await;
}

/// A utility's file operand `/dev/null` and `/dev/zero` are the devices on
/// every system, Windows included, as in Git Bash: `/dev/null` reads
/// nothing and takes what is written, `/dev/zero` reads zeros. Before, on
/// Windows each named a missing file. A utility that reads the operand's
/// metadata first (cat, head, tail, diff, rg, ls, du, cp's source) still
/// fails on Windows, which gives the null device none, and is left out.
/// Runs in about 0.1 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dev_null_and_dev_zero_are_devices_for_utilities() {
    expect_cases(
        "echo hi > x",
        &[
            ("wc -l /dev/null", 0, "0 /dev/null\n", ""),
            ("sort /dev/null x", 0, "hi\n", ""),
            ("uniq /dev/null", 0, "", ""),
            ("cut -c 1 /dev/null", 0, "", ""),
            ("paste /dev/null x", 0, "\thi\n", ""),
            ("nl /dev/null", 0, "", ""),
            ("tac /dev/null", 0, "", ""),
            ("od -An /dev/null", 0, "", ""),
            ("cmp /dev/null /dev/null", 0, "", ""),
            ("grep -c hi /dev/null", 1, "0\n", ""),
            ("sed p /dev/null", 0, "", ""),
            ("jq . /dev/null", 0, "", ""),
            ("cp x /dev/null && cat x", 0, "hi\n", ""),
            ("tee /dev/null < x", 0, "hi\n", ""),
            ("sort -o /dev/null x", 0, "", ""),
            ("uniq x /dev/null", 0, "", ""),
            ("od -An -tx1 -N 2 /dev/zero", 0, " 00 00\n", ""),
            ("cmp -n 8 /dev/zero /dev/zero", 0, "", ""),
        ],
    )
    .await;
}

/// A utility whose reader has closed the pipe ends with 141 and nothing on
/// stderr, as SIGPIPE ends GNU's in a shell: `diff` panicked, `cmp` and the
/// coreutils printed "Broken pipe" and exited 1 or 2, and `du`'s walk
/// reported its printer's end as "sending on a closed channel". Each
/// output is well over 128 KiB, what a 64 KiB pipe and `head`'s first read
/// of up to 64 KiB take together, so the writer still has bytes to write
/// once its reader is gone, however the threads are scheduled: `sort` sorts
/// `l2 l1` (244 KB), since `l1` alone (122 KB) fit in the pipe after `head`'s
/// read and often ended with 0 on Linux; `cmp -l` lists each differing
/// byte, so it compares `l1` with `l0`, which differs in every zero
/// (1.7 MB), not with `l2`, which differs in one byte a line (30 KB). On
/// Linux, `cat`'s splice path grew its output pipe to 1 MiB before it found
/// splice off in a job, so `cat l1 l1 l1` (366 KB) fit and ended with 0.
/// `rg 0 .` searches its files in parallel: before, its workers' end on the
/// closed pipe panicked the walk (`rg: utility thread panicked`). Runs in
/// about 1.7 s; the `rg` case added 0.7 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_pipe_ends_utilities_quietly_as_sigpipe_does() {
    let setup = "seq -f '%060g' 1 2000 > l1; seq -f '%060g' 2001 4000 > l2; tr 0 1 < l1 > l0; \
                 mkdir t; seq -f 't/%060g' 1 3000 | xargs touch";
    let cases: Vec<(String, u8, &str, &str)> = [
        "diff l1 l2", "diff -u l1 l2", "diff -y l1 l2", "cmp -l l1 l0", "cat l1 l1 l1", "sort l2 l1", "du -a t",
        "rg 0 .",
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
                 printf '%4000s' '' | tr ' ' a > b4; printf '\\0' >> b4; \
                 printf '%5000s' '' | tr ' ' a > b5; printf '\\0\\n' >> b5; printf 'x\\n' > c";
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

/// `grep` treats a file as GNU grep 3.12 does in a UTF-8 locale: a NUL
/// anywhere in what it has read of a file makes the file binary before any
/// of its lines is printed, and a selected line that is not UTF-8 is held
/// back while the others print. Without `-I` each such file that matches
/// gets `binary file matches` on stderr; `-I` skips a file with a NUL as
/// one without a match and drops the lines that are not UTF-8 silently.
/// Before, a matching line before the NUL printed, and under `-I` a line
/// that was not UTF-8 printed raw, which made a job's whole stdout binary.
/// The file with a NUL is `zeroed`, as `nul` names a device on Windows.
/// `five` takes the literal fast path, `f.ve` the line-at-a-time one. The
/// expected outputs are GNU grep 3.12's. Runs in about 0.05 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grep_treats_binary_files_as_gnu_grep_does() {
    let setup = "printf 'five\\n\\377 five\\nfive again\\n' > enc; printf 'five\\nsix\\n\\0\\n' > zeroed; \
                 printf '%40000s' '' | tr ' ' a > latenul; printf '\\nfive\\n\\0\\n' >> latenul; \
                 printf 'five\\n' > plain";
    let lines = "enc:1:five\nenc:3:five again\nplain:1:five\n";
    let notices = "grep: enc: binary file matches\ngrep: zeroed: binary file matches\n\
                   grep: latenul: binary file matches\n";
    expect_cases(
        setup,
        &[
            ("grep -n five enc zeroed latenul plain", 0, lines, notices),
            ("grep -n f.ve enc zeroed latenul plain", 0, lines, notices),
            ("grep -n -I five enc zeroed latenul plain", 0, lines, ""),
            ("grep -n --binary-files=without-match f.ve enc zeroed latenul plain", 0, lines, ""),
            ("grep -I -c five enc zeroed latenul", 0, "enc:3\nzeroed:0\nlatenul:0\n", ""),
            ("grep -I -L five zeroed plain", 0, "zeroed\n", ""),
            ("grep -I -l five zeroed", 1, "", ""),
            ("grep -o five enc", 0, "five\nfive\nfive\n", ""),
            ("grep -c five enc zeroed", 0, "enc:3\nzeroed:1\n", ""),
            ("grep -a -c five zeroed latenul", 0, "zeroed:1\nlatenul:1\n", ""),
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
