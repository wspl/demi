//! Starting with the computer (`runner.md` § Starting with the computer):
//! the installation's registration with the service manager the system
//! offers its user, which starts the runner at the user's login and again
//! after a crash. Every runner of an installer's installation makes sure of
//! it as it starts, and one the manager did not start hands the
//! installation over to the manager's; `run start` asks the manager for the
//! runner; a removal removes the registration last.
//!
//! The files the managers read are written directly; the managers are asked
//! through their own commands, `launchctl`, `systemctl` and `loginctl`, and
//! on Windows PowerShell's ScheduledTasks module, since none offers its user
//! a library interface (the Task Scheduler's COM interface aside, which
//! PowerShell's module wraps); the `HKCU\…\Run` value is written through
//! the registry's API.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

#[cfg(unix)]
use demi_shared_artifacts::{Mode, Permissions, Publication};
use demi_runner_protocol::{SUPERVISED, wire::AutostartUnavailable};

#[cfg(feature = "test-fixtures")]
use demi_runner_protocol::SERVICE_MANAGER_STAND_IN as STAND_IN;

/// What a registration that is in place does for the runner.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Supervision {
    /// The manager starts it at login and again after a crash, and starts
    /// it when asked.
    Restarts,
    /// Windows starts it at logon from the `Run` value, and nothing starts
    /// it again.
    #[cfg(windows)]
    AtLogon,
}

/// The registration of one installation, named by its directory's name, so
/// that the runners of several backends on one computer are registered
/// apart.
pub struct Registration {
    id: String,
    /// The installation's directory, absolute.
    directory: PathBuf,
    home: PathBuf,
    /// Whether this process is the runner the manager started.
    supervised: bool,
}

impl Registration {
    /// The registration of the installation in `directory` of the user whose
    /// home is `home`, as this process sees it.
    pub fn of(directory: &Path, home: &Path) -> io::Result<Self> {
        let directory = std::path::absolute(directory)?;
        let id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::other("the installation's directory has no name"))?
            .to_owned();
        Ok(Self {
            id,
            directory,
            home: home.to_owned(),
            supervised: std::env::var_os(SUPERVISED).is_some_and(|value| !value.is_empty()),
        })
    }

    /// Whether this process is the runner the manager started, which never
    /// hands over.
    pub fn supervised(&self) -> bool {
        self.supervised
    }

    /// Makes sure the registration exists and names this installation, and
    /// writes it when it does not: what it does for the runner, or why none
    /// can be made. What goes wrong while a registration stays in place,
    /// such as a manager that cannot load it now, is logged only.
    pub async fn ensure(&self) -> Result<Supervision, AutostartUnavailable> {
        // The managers' names allow these characters, and the installers
        // name an installation with them alone.
        let named = self
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !named {
            return Err(AutostartUnavailable::UnusableName {
                name: self.id.clone(),
            });
        }
        manager::ensure(self).await
    }

    /// Whether the registration exists, so that the manager starts the
    /// runner.
    pub async fn exists(&self) -> bool {
        manager::exists(self).await
    }

    /// Asks the manager to start the installation's runner, again when one
    /// of its own is on its way out.
    pub async fn start(&self) -> Result<(), String> {
        manager::start(self).await
    }

    /// Removes the registration, so the manager never starts a runner that
    /// is gone; what stays is said to the person at the console.
    pub async fn remove(&self) {
        if let Err(error) = manager::remove(self).await {
            crate::console::line(crate::console::runner_line(format_args!(
                "the registration that starts this runner with the computer stays: {error}"
            )));
        }
    }

    /// The file `path` could not be written, for `error`.
    #[cfg(unix)]
    fn not_written(path: &Path, error: &io::Error) -> AutostartUnavailable {
        AutostartUnavailable::NotWritten {
            path: path.to_string_lossy().into_owned(),
            error: error.to_string(),
        }
    }
}

/// The account's name, as the user database names it.
#[cfg(target_os = "linux")]
fn user_name() -> Option<String> {
    let user = uzers::get_user_by_uid(uzers::get_current_uid())?;
    user.name().to_str().map(str::to_owned)
}

/// One of the service manager's commands, or its stand-in in a test.
fn system(program: &str) -> tokio::process::Command {
    #[cfg(feature = "test-fixtures")]
    if let Some(stand_in) = std::env::var_os(STAND_IN) {
        let mut command = tokio::process::Command::new(stand_in);
        command.arg(program);
        return command;
    }
    tokio::process::Command::new(program)
}

/// Runs `command`, named `program`: its standard output, or why it failed,
/// which its standard error says.
async fn run(program: &str, mut command: tokio::process::Command) -> Result<String, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // No console window of its own for a runner that has none.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .await
        .map_err(|error| format!("{program} did not start: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.trim();
    Err(if said.is_empty() {
        format!("{program} ended with {}", output.status)
    } else {
        format!("{program}: {said}")
    })
}

/// Writes `contents` to `path` unless it holds them already, as the owner's
/// alone; whether it wrote.
#[cfg(unix)]
async fn write(path: &Path, contents: &str) -> io::Result<bool> {
    match tokio::fs::read(path).await {
        Ok(found) if found == contents.as_bytes() => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    demi_shared_artifacts::publish_bytes(
        path,
        contents.as_bytes(),
        Publication {
            mode: Mode::Replace,
            permissions: Permissions::Private,
            durable: true,
        },
    )
    .await
    .map_err(io::Error::other)?;
    Ok(true)
}

/// Removes the file at `path`; one that is not there is removed already.
#[cfg(unix)]
async fn remove_file(path: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `text` with the five characters XML gives a meaning escaped.
#[cfg(any(target_os = "macos", windows))]
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The agent's definition: the launcher with `run --log`, named the
/// manager's runner by [`SUPERVISED`], at the user's login, again after a
/// crash or a non-zero exit at most every 10 s, as a process the system
/// does not throttle; its output goes nowhere, which is launchd's default.
#[cfg(target_os = "macos")]
fn plist(label: &str, launcher: &str, home: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{launcher}</string>
		<string>run</string>
		<string>--log</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>{SUPERVISED}</key>
		<string>1</string>
	</dict>
	<key>WorkingDirectory</key>
	<string>{home}</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ThrottleInterval</key>
	<integer>10</integer>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
        label = xml(label),
        launcher = xml(launcher),
        home = xml(home),
    )
}

/// `value` as one word of a systemd command line, and with nothing of
/// its own expanded.
#[cfg(target_os = "linux")]
fn word(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// The unit: the launcher with `run --log`, named the manager's runner by
/// [`SUPERVISED`], at boot with lingering or at the user's first login,
/// again 10 s after a failure; its output goes nowhere, since the runner
/// writes its own log.
#[cfg(target_os = "linux")]
fn unit(launcher: &str, home: &str, directory: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Demi runner of {directory}\n\
         \n\
         [Service]\n\
         ExecStart={launcher} run --log\n\
         Environment={SUPERVISED}=1\n\
         WorkingDirectory={home}\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         StandardOutput=null\n\
         StandardError=null\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        directory = directory.replace('%', "%%"),
        launcher = word(launcher),
        home = word(home),
    )
}

/// The task: at the user's logon, with the user's limited rights, hidden;
/// started again a minute after a failure, the shortest the Task Scheduler
/// takes; a start while an instance runs waits for it to end, so the
/// manager's runner a successor hands over to starts once the task's
/// earlier instance, ending, let go; never stopped for its running time, a
/// battery or idleness; at normal priority, where a task's default is below
/// it.
#[cfg(windows)]
fn task(user: &str, description: &str, arguments: &str, home: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Queue</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>powershell.exe</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{home}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        description = xml(description),
        user = xml(user),
        arguments = xml(arguments),
        home = xml(home),
    )
}

/// A LaunchAgent in the user's `gui` domain.
#[cfg(target_os = "macos")]
mod macos {
    use super::{AutostartUnavailable, Registration, Supervision, plist, run, system, write};
    use std::path::PathBuf;

    impl Registration {
        fn label(&self) -> String {
            format!("sh.demi.runner.{}", self.id)
        }

        fn plist(&self) -> PathBuf {
            self.home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{}.plist", self.label()))
        }

        fn domain() -> String {
            format!("gui/{}", rustix::process::getuid().as_raw())
        }

        fn service(&self) -> String {
            format!("{}/{}", Self::domain(), self.label())
        }
    }

    async fn launchctl(arguments: &[&str]) -> Result<String, String> {
        let mut command = system("launchctl");
        command.args(arguments);
        run("launchctl", command).await
    }

    pub async fn ensure(registration: &Registration) -> Result<Supervision, AutostartUnavailable> {
        let path = registration.plist();
        let contents = plist(
            &registration.label(),
            &registration.directory.join("run").to_string_lossy(),
            &registration.home.to_string_lossy(),
        );
        let written = write(&path, &contents)
            .await
            .map_err(|error| Registration::not_written(&path, &error))?;
        let service = registration.service();
        let loaded = launchctl(&["print", &service]).await.is_ok();
        if loaded && !written {
            return Ok(Supervision::Restarts);
        }
        // The agent runs this process: launchd reads the new definition at
        // the next login, and a reload now would stop this runner.
        if loaded && registration.supervised {
            return Ok(Supervision::Restarts);
        }
        // An agent loaded with another definition, such as one naming
        // another installation of this name, is loaded again.
        if loaded && let Err(error) = launchctl(&["bootout", &service]).await {
            tracing::warn!("the runner's LaunchAgent could not be unloaded to load its new definition: {error}");
            return Ok(Supervision::Restarts);
        }
        // Loading it starts the manager's runner at once, which finds the
        // lock held and exits until this runner hands over. A domain that
        // cannot load it now, as without the user's login session, loads it
        // at the next login.
        if let Err(error) = launchctl(&["bootstrap", &Registration::domain(), &path.to_string_lossy()]).await {
            tracing::warn!("the runner's LaunchAgent is in place and loads at the next login; it could not be loaded now: {error}");
        }
        Ok(Supervision::Restarts)
    }

    pub async fn exists(registration: &Registration) -> bool {
        tokio::fs::try_exists(registration.plist()).await.unwrap_or(false)
    }

    pub async fn start(registration: &Registration) -> Result<(), String> {
        // `-k` starts the agent's runner again when one is on its way out,
        // as the one loading the agent started, which found the lock held.
        // An agent that is not loaded starts as it loads.
        if launchctl(&["kickstart", "-k", &registration.service()]).await.is_ok() {
            return Ok(());
        }
        launchctl(&[
            "bootstrap",
            &Registration::domain(),
            &registration.plist().to_string_lossy(),
        ])
        .await
        .map(drop)
    }

    pub async fn remove(registration: &Registration) -> Result<(), String> {
        let path = registration.plist();
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(());
        }
        super::remove_file(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut bootout = system("launchctl");
        bootout.args(["bootout", &registration.service()]);
        // Unloading the agent ends the process it runs, after waiting for
        // it: when that is this one, which ends next, nobody waits. The
        // command runs in a process group of its own, since launchd ends
        // the rest of the job's group as this process exits.
        if registration.supervised {
            bootout
                .process_group(0)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            return bootout.spawn().map(drop).map_err(|error| format!("launchctl did not start: {error}"));
        }
        // An agent that is not loaded leaves nothing to unload.
        let _ = run("launchctl", bootout).await;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
use macos as manager;

/// A systemd user unit, with lingering for the user.
#[cfg(target_os = "linux")]
mod linux {
    use super::{AutostartUnavailable, Registration, Supervision, run, system, unit, write};
    use std::path::{Path, PathBuf};

    impl Registration {
        fn unit_name(&self) -> String {
            format!("demi-runner-{}.service", self.id)
        }

        /// In the user's configuration, where the user manager reads it.
        fn unit(&self) -> PathBuf {
            self.home
                .join(".config")
                .join("systemd")
                .join("user")
                .join(self.unit_name())
        }
    }

    /// `systemctl --user` with `arguments`: the user's manager answers at
    /// the private socket in the user's runtime directory, which a process
    /// started without a login session may not name.
    async fn systemctl(arguments: &[&str]) -> Result<String, String> {
        let mut command = system("systemctl");
        command.arg("--user").args(arguments);
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            let runtime = PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()));
            if runtime.is_dir() {
                command.env("XDG_RUNTIME_DIR", runtime);
            }
        }
        run("systemctl", command).await
    }

    /// Whether systemd runs the system, as `sd_booted` tells.
    fn booted() -> bool {
        #[cfg(feature = "test-fixtures")]
        if std::env::var_os(super::STAND_IN).is_some() {
            return true;
        }
        Path::new("/run/systemd/system").is_dir()
    }

    pub async fn ensure(registration: &Registration) -> Result<Supervision, AutostartUnavailable> {
        if !booted() {
            return Err(AutostartUnavailable::NoSystemd);
        }
        let path = registration.unit();
        let contents = unit(
            &registration.directory.join("run").to_string_lossy(),
            &registration.home.to_string_lossy(),
            &registration.directory.to_string_lossy(),
        );
        let written = write(&path, &contents)
            .await
            .map_err(|error| Registration::not_written(&path, &error))?;
        let unanswered = |error: String| AutostartUnavailable::NoUserManager { error };
        if written {
            systemctl(&["daemon-reload"]).await.map_err(unanswered)?;
        }
        systemctl(&["enable", &registration.unit_name()])
            .await
            .map_err(unanswered)?;
        let Some(user) = super::user_name() else {
            return Err(AutostartUnavailable::UnknownAccount);
        };
        if Path::new("/var/lib/systemd/linger").join(&user).exists() {
            return Ok(Supervision::Restarts);
        }
        let mut linger = system("loginctl");
        linger.arg("enable-linger");
        match run("loginctl", linger).await {
            Ok(_) => Ok(Supervision::Restarts),
            Err(error) => {
                tracing::warn!("lingering could not be turned on: {error}");
                Err(AutostartUnavailable::LingerOff { user })
            }
        }
    }

    pub async fn exists(registration: &Registration) -> bool {
        tokio::fs::try_exists(registration.unit()).await.unwrap_or(false)
    }

    pub async fn start(registration: &Registration) -> Result<(), String> {
        // `restart` starts the unit's runner again when one is on its way
        // out, and starts it when none runs.
        systemctl(&["restart", &registration.unit_name()]).await.map(drop)
    }

    pub async fn remove(registration: &Registration) -> Result<(), String> {
        let path = registration.unit();
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(());
        }
        // Disabling stops nothing: the runner ends next, and with 0 its unit
        // is not started again. A manager that does not answer has no link
        // to remove, and forgets the unit once its file is gone.
        let _ = systemctl(&["disable", &registration.unit_name()]).await;
        super::remove_file(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let _ = systemctl(&["daemon-reload"]).await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
use linux as manager;

/// A scheduled task with a logon trigger for the user, else the `Run`
/// value of the user's registry.
#[cfg(windows)]
mod windows {
    use super::{AutostartUnavailable, Registration, SUPERVISED, Supervision, run, system, task};

    /// The key whose values Windows starts at the user's logon.
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    impl Registration {
        /// The name of the task's installation, which each PowerShell
        /// script below follows with the user's SID: Windows keeps every
        /// user's tasks in one folder. The `Run` value is the user's own.
        fn name(&self) -> String {
            format!("sh.demi.runner.{}", self.id)
        }

        /// PowerShell's arguments, which run the launcher with `run --log`,
        /// hidden, as the manager's runner.
        fn arguments(&self) -> String {
            let launcher = self.directory.join("run.ps1").to_string_lossy().replace('\'', "''");
            format!(
                "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -Command \"$env:{SUPERVISED} = '1'; & '{launcher}' run --log; exit $LASTEXITCODE\""
            )
        }

        fn description(&self) -> String {
            format!("Starts the Demi runner of {} at logon.", self.directory.display())
        }
    }

    /// What the task's XML says in place of the user, which [`REGISTER`]
    /// replaces with the security identifier of the user this process runs
    /// as: the environment's `USERDOMAIN` and `USERNAME` may name no
    /// account, as `WORKGROUP\ana` named none on a Windows 11 that a
    /// runner served.
    const USER: &str = "DEMI-TASK-USER";

    /// What every script begins with: the task's name, the installation's
    /// followed by the user's SID.
    const NAMED: &str = "$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value; \
        $name = $env:DEMI_TASK_NAME + '.' + $sid; ";

    /// Registers the task `DEMI_TASK_XML` describes, for this process's
    /// user, unless one of its name already runs the same program for the
    /// same installation.
    const REGISTER: &str = "$ErrorActionPreference = 'Stop'; \
        $task = Get-ScheduledTask -TaskPath '\\' -TaskName $name -ErrorAction SilentlyContinue; \
        if ($task -and $task.Description -eq $env:DEMI_TASK_DESCRIPTION -and @($task.Actions).Count -eq 1 -and \
          $task.Actions[0].Execute -eq 'powershell.exe' -and $task.Actions[0].Arguments -eq $env:DEMI_TASK_ARGUMENTS -and \
          $task.Settings.MultipleInstances -eq 'Queue') { exit 0 }; \
        Register-ScheduledTask -TaskName $name -Xml $env:DEMI_TASK_XML.Replace($env:DEMI_TASK_USER, $sid) -Force | Out-Null";

    async fn powershell(script: &str, registration: &Registration, extra: &[(&str, String)]) -> Result<String, String> {
        let mut command = system("powershell.exe");
        command
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &format!("{NAMED}{script}")])
            .env("DEMI_TASK_NAME", registration.name());
        for (name, value) in extra {
            command.env(name, value);
        }
        run("PowerShell", command).await
    }

    pub async fn ensure(registration: &Registration) -> Result<Supervision, AutostartUnavailable> {
        let description = registration.description();
        let arguments = registration.arguments();
        let task = task(USER, &description, &arguments, &registration.home.to_string_lossy());
        let extra = [
            ("DEMI_TASK_USER", USER.to_owned()),
            ("DEMI_TASK_XML", task),
            ("DEMI_TASK_DESCRIPTION", description),
            ("DEMI_TASK_ARGUMENTS", arguments.clone()),
        ];
        match powershell(REGISTER, registration, &extra).await {
            Ok(_) => {
                // The task starts the runner; a `Run` value of before would
                // start a second one.
                if let Err(error) = run_value::remove(&registration.name()) {
                    tracing::warn!("the runner's Run value stays beside its scheduled task: {error}");
                }
                Ok(Supervision::Restarts)
            }
            Err(refused) => {
                let command = format!("powershell.exe {arguments}");
                run_value::set(&registration.name(), &command).map_err(|error| {
                    AutostartUnavailable::Refused {
                        error: format!("{refused}; {error}"),
                    }
                })?;
                tracing::warn!("the runner starts at logon from its Run value, and not again after a crash: Windows refused its scheduled task: {refused}");
                Ok(Supervision::AtLogon)
            }
        }
    }

    pub async fn exists(registration: &Registration) -> bool {
        const FIND: &str = "Get-ScheduledTask -TaskPath '\\' -TaskName $name -ErrorAction Stop | Out-Null";
        powershell(FIND, registration, &[]).await.is_ok()
    }

    pub async fn start(registration: &Registration) -> Result<(), String> {
        // A start while the task's instance runs waits for it to end.
        const START: &str = "Start-ScheduledTask -TaskPath '\\' -TaskName $name -ErrorAction Stop";
        powershell(START, registration, &[]).await.map(drop)
    }

    pub async fn remove(registration: &Registration) -> Result<(), String> {
        const UNREGISTER: &str = "$task = Get-ScheduledTask -TaskPath '\\' -TaskName $name -ErrorAction SilentlyContinue; \
            if ($task) { Unregister-ScheduledTask -TaskPath '\\' -TaskName $name -Confirm:$false -ErrorAction Stop }; \
            exit 0";
        let task = powershell(UNREGISTER, registration, &[]).await.map(drop);
        let value = run_value::remove(&registration.name()).map_err(|error| error.to_string());
        task.and(value)
    }

    /// The value of the user's `Run` key that starts the runner at logon.
    mod run_value {
        use super::RUN_KEY;
        use std::io;
        use windows_sys::Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW},
        };

        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().chain([0]).collect()
        }

        pub fn set(name: &str, command: &str) -> io::Result<()> {
            let (key, name, data) = (wide(RUN_KEY), wide(name), wide(command));
            let size = u32::try_from(data.len() * 2).map_err(io::Error::other)?;
            // SAFETY: the key, the name and the data are NUL-terminated
            // UTF-16 strings that outlive the call, and `size` is the
            // data's size in bytes with its NUL.
            let status = unsafe {
                RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), size)
            };
            match status {
                ERROR_SUCCESS => Ok(()),
                status => Err(io::Error::from_raw_os_error(status as i32)),
            }
        }

        pub fn remove(name: &str) -> io::Result<()> {
            let (key, name) = (wide(RUN_KEY), wide(name));
            // SAFETY: the key and the name are NUL-terminated UTF-16
            // strings that outlive the call.
            let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
            match status {
                ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
                status => Err(io::Error::from_raw_os_error(status as i32)),
            }
        }
    }
}

#[cfg(windows)]
use windows as manager;
