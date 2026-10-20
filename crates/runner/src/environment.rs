//! The device environment: the user's login environment, which the runner reads
//! at its start from the account's login shell, run as an interactive login
//! shell as VS Code resolves a shell's environment.
//!
//! A service manager gives the runner a small environment. On macOS the user's
//! `PATH`, Homebrew's, nvm's, lives in zsh's files, which a job's login shell
//! never reads. So the environment comes from the user's shell, not from
//! whoever started the runner, the same after a restart as from a terminal.
//!
//! A shell that fails or takes over 10 s leaves the runner's own, and the log
//! says so.
//!
//! # What is removed and added
//!
//! `DEMI_*` names and, on Windows, the `=C:`-style per-drive entries are taken
//! away:
//!
//! - The `DEMI_*` names configure the runner or belong to whatever started it,
//!   and none is meant for the programs it starts. A job that reads
//!   `DEMI_RELEASE_ID` to choose a release would otherwise read the runner's.
//! - cmd keeps the per-drive entries, and no program may be given them back.
//!
//! The same environment reaches jobs, raw processes and resident services.
//! Runner-owned variables (the endpoint, the context handle, `DEMI_HOME`, the
//! alias directory on `PATH`) override caller values.
//!
//! On macOS the `LANG` Terminal sets and no profile does is completed
//! ([`complete`]) from the user's language with UTF-8, since `ls` and `git`
//! print a non-ASCII file name as question marks without it.

use std::collections::BTreeMap;

/// What the login shell is given to print the environment it ends up with:
/// this program with [`PRINT`] and a mark, which it writes before and after
/// the environment as JSON.
pub const PRINT: &str = "environment";

/// Set while the login shell runs, so that a user's shell file can tell
/// this start from a person's, as `VSCODE_RESOLVING_ENVIRONMENT` does; a
/// `DEMI_` name, so the device environment never carries it.
#[cfg(unix)]
const RESOLVING: &str = "DEMI_RESOLVING_ENVIRONMENT";

/// How long the login shell may take.
#[cfg(unix)]
const LOGIN_SHELL: std::time::Duration = std::time::Duration::from_secs(10);

/// Prints the environment of this process between `mark`s, for
/// [`login_environment`].
pub fn print(mark: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let json = serde_json::to_string(&env).map_err(std::io::Error::other)?;
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "{mark}{json}{mark}")?;
    stdout.flush()
}

/// The environment the account's login shell ends up with: `SHELL` of
/// `own`, this process's environment, or the account's entry in the user
/// database when unset, run as an interactive login shell with `own`; or
/// why there is none, after which the runner keeps `own`. The shell is
/// named in the result's `SHELL` when its files left it unset.
#[cfg(unix)]
pub async fn login_environment(
    own: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    use std::os::unix::process::CommandExt as _;
    let shell = own
        .get("SHELL")
        .filter(|shell| !shell.is_empty())
        .cloned()
        .or_else(account_shell)
        .ok_or("neither SHELL nor the user database names a login shell")?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("this program's path is unknown: {error}"))?;
    let executable = executable.to_string_lossy();
    // In single quotes: a POSIX shell's, and PowerShell's, which doubles a
    // quote within.
    let quoted = executable.replace('\'', "'\\''");
    let mark = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let name = std::path::Path::new(&shell)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The shells VS Code knows apart: PowerShell and nushell quote a
    // program's path their own way, and csh takes no `-l` with `-c`.
    let (arguments, script): (&[&str], String) = match name.as_str() {
        "pwsh" | "pwsh-preview" | "powershell" => (
            &["-Login", "-Command"],
            format!("& '{}' {PRINT} {mark}", executable.replace('\'', "''")),
        ),
        "nu" => (&["-i", "-l", "-c"], format!("^'{quoted}' {PRINT} {mark}")),
        "csh" | "tcsh" => (&["-ic"], format!("'{quoted}' {PRINT} {mark}")),
        _ => (&["-i", "-l", "-c"], format!("'{quoted}' {PRINT} {mark}")),
    };
    let mut command = {
        let mut command = std::process::Command::new(&shell);
        command
            .args(arguments)
            .arg(script)
            .env_clear()
            .envs(own)
            .env(RESOLVING, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        // SAFETY: the hook runs in the child between fork and exec and makes
        // only a `setsid` call, which is async-signal-safe, and allocates
        // nothing. A session of its own has no controlling terminal, which
        // an interactive shell would otherwise take over, and its group is
        // the one a timeout ends.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                Ok(())
            });
        }
        tokio::process::Command::from(command)
    };
    command.kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|error| format!("the login shell {shell} did not start: {error}"))?;
    let group = child
        .id()
        .and_then(|id| rustix::process::Pid::from_raw(id.cast_signed()));
    let output = match tokio::time::timeout(timeout(), child.wait_with_output()).await {
        Ok(output) => output.map_err(|error| format!("the login shell {shell} failed: {error}"))?,
        Err(_) => {
            // What the shell started goes with it; a group that ended
            // already has nothing to end.
            if let Some(group) = group {
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            }
            return Err(format!(
                "the login shell {shell} took longer than {} s",
                timeout().as_secs_f32()
            ));
        }
    };
    if !output.status.success() {
        return Err(format!("the login shell {shell} ended with {}", output.status));
    }
    let printed = String::from_utf8_lossy(&output.stdout);
    let json = printed
        .split(&*mark)
        .nth(1)
        .filter(|_| printed.matches(&*mark).count() >= 2)
        .ok_or_else(|| format!("the login shell {shell} printed no environment"))?;
    let mut env: BTreeMap<String, String> = serde_json::from_str(json)
        .map_err(|error| format!("the login shell {shell} printed an environment that does not decode: {error}"))?;
    env.remove(RESOLVING);
    env.entry("SHELL".into()).or_insert(shell);
    Ok(env)
}

/// How long the login shell may take: [`LOGIN_SHELL`], shorter in a test.
#[cfg(unix)]
fn timeout() -> std::time::Duration {
    #[cfg(feature = "test-fixtures")]
    if let Some(milliseconds) = std::env::var("DEMI_LOGIN_SHELL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        return std::time::Duration::from_millis(milliseconds);
    }
    LOGIN_SHELL
}

/// The account's login shell, as the user database names it.
#[cfg(unix)]
fn account_shell() -> Option<String> {
    use uzers::os::unix::UserExt as _;
    let user = uzers::get_user_by_uid(uzers::get_current_uid())?;
    let shell = user.shell().to_str()?;
    (!shell.is_empty()).then(|| shell.to_owned())
}

/// Sets `LANG` in `env` when unset, as Terminal does on macOS.
pub fn complete(env: &mut BTreeMap<String, String>) {
    #[cfg(target_os = "macos")]
    if !env.contains_key("LANG") {
        env.insert("LANG".into(), macos::lang());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = env;
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;

    /// The locales the system has.
    const LOCALES: &str = "/usr/share/locale";

    /// The user's language and region with UTF-8, from the region the user
    /// chose, `AppleLocale`, such as `zh_CN` or `en_GB@rg=gbzzzz`; when the
    /// system has no such locale, such as `en_CN`, the first it has of the
    /// same language; else `en_US.UTF-8`.
    pub fn lang() -> String {
        use objc2_core_foundation::{CFPreferencesCopyAppValue, CFString, kCFPreferencesAnyApplication};
        // SAFETY: the constant is CoreFoundation's, set before any code
        // runs, and never changes.
        let any = unsafe { kCFPreferencesAnyApplication };
        let locale = CFPreferencesCopyAppValue(&CFString::from_str("AppleLocale"), any)
            .and_then(|value| value.downcast::<CFString>().ok())
            .map(|locale| locale.to_string());
        // A test names the region the user chose.
        #[cfg(feature = "test-fixtures")]
        let locale = std::env::var("DEMI_APPLE_LOCALE").ok().or(locale);
        locale
            .as_deref()
            .and_then(|locale| chosen(locale, Path::new(LOCALES)))
            .unwrap_or_else(|| "en_US.UTF-8".into())
    }

    /// The UTF-8 locale of `locale` that `locales` has, else the first it
    /// has of its language.
    fn chosen(locale: &str, locales: &Path) -> Option<String> {
        let locale = locale.split('@').next()?;
        let (language, region) = locale.split_once('_')?;
        // A language with its script, such as `zh-Hans`, by its language.
        let language = language.split('-').next()?;
        let exact = format!("{language}_{region}.UTF-8");
        if locales.join(&exact).is_dir() {
            return Some(exact);
        }
        let prefix = format!("{language}_");
        let mut found: Vec<String> = std::fs::read_dir(locales)
            .ok()?
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with(&prefix) && name.ends_with(".UTF-8"))
            .collect();
        found.sort();
        found.into_iter().next()
    }
}
