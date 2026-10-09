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
