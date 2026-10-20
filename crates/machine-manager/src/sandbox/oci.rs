//! One boot's OCI runtime configuration: the prepared root, UID 1000's process
//! under init, the runtime mounts, the network namespace and, with the resource
//! limits on, the cgroup and its limits. The profile is code, not configuration:
//! only the paths, the slot and the limits vary, and a workload cannot supply
//! annotations, host paths, runtime flags or network or cgroup identities.
//!
//! The isolation boundary is the gVisor sandbox, not a workspace, a Docker
//! namespace or a hardware VM: each user gets a separate Sentry, filesystem view,
//! network namespace and, with the limits on, resource group. What stays trusted
//! is the Linux kernel, the manager, the `runsc` distribution and the image
//! builder. Operations the product does not support return errors; they never
//! widen isolation, and the profile changes only with a design and acceptance
//! update, never as a per-user escape hatch. The product supports ordinary Linux
//! development and conversation browsers, not arbitrary kernel features or
//! privileged nested Docker.
//!
//! - `noNewPrivileges` is off so that sudo works. The initial process has empty
//!   effective, permitted, inheritable and ambient sets; setuid programs may
//!   gain only what the bounding set lists, which excludes host administration,
//!   module loading, raw devices and network administration. Test sudo and
//!   Chrome with exactly this set; Chrome runs without its own sandbox on Linux.
//! - Only the prepared root, this device's home, the runtime mounts and the
//!   directory of the backend's runner socket reach the Gofer: no checkout, Mac
//!   home, Docker socket, manager state, arbitrary host path or other user's
//!   mount. `/proc` and the devices are sandbox views, and the host's sysfs and
//!   writable cgroups are not mounted. No privileged mode, host networking,
//!   unconfined runtime seccomp, host PID namespace or runtime socket is exposed,
//!   and `runsc`'s standard Directfs profile stays.
//! - The runtime's inputs are pinned in `runtime/release.json`: upstream's
//!   gVisor release, the same version on amd64 and arm64, and the SHA-512 of each
//!   architecture's archive. The manager is built with that manifest and refuses
//!   to start unless the configured `runsc` reports exactly the pinned version.
//!   A new pin passes the acceptance and the probes of the bugs earlier versions
//!   had (`runtime/README.md`) and changes no user storage format.

use std::{collections::HashSet, path::Path};

use demi_machine_manager_protocol::image::{INIT_PATH, PROGRAMS_PATH, RUNNER_PATH};
use demi_runner_protocol::{
    boot::{BACKEND_SOCKET, TEMPORARY_DIRECTORY},
    release::RELEASE_ENV,
};
use oci_spec::{
    OciSpecError,
    runtime::{
        Capability, LinuxBuilder, LinuxCapabilitiesBuilder, LinuxCpuBuilder, LinuxMemoryBuilder,
        LinuxNamespaceBuilder, LinuxNamespaceType, LinuxPidsBuilder, LinuxResourcesBuilder, Mount,
        MountBuilder, PosixRlimitBuilder, PosixRlimitType, ProcessBuilder, RootBuilder, Spec,
        SpecBuilder, UserBuilder,
    },
};

use super::files::{RuntimeDirectory, USER_ID};
use crate::config::Limits;

/// The capabilities setuid programs such as sudo may gain; the process
/// itself starts with none.
const BOUNDING: [Capability; 11] = [
    Capability::Chown,
    Capability::DacOverride,
    Capability::Fowner,
    Capability::Fsetid,
    Capability::Kill,
    Capability::Setgid,
    Capability::Setuid,
    Capability::Setpcap,
    Capability::NetBindService,
    Capability::SysChroot,
    Capability::Setfcap,
];

/// Where the runner reads its boot record inside the sandbox.
pub const BOOT_RECORD: &str = "/run/demi-boot.json";

/// The CFS period the CPU budget is a quota of.
const CPU_PERIOD: u64 = 100_000;

/// The sandbox's process limit, host PIDs and ordinary-user processes alike.
const PROCESS_LIMIT: u64 = 1024;

/// What one boot's configuration varies by.
pub struct Boot<'a> {
    pub directory: &'a RuntimeDirectory,
    /// The network namespace's name under `/run/netns`.
    pub namespace: &'a str,
    /// The boot's cgroup; `None` with the resource limits off, when the
    /// sandbox has none.
    pub cgroup: Option<Cgroup<'a>>,
    /// The release of the runner the sandbox runs, which the runner names
    /// to the backend (`crates/runner/src/update.rs`).
    pub runner_release: &'a str,
    /// The directory of the backend's runner socket, which the sandbox sees
    /// where its runner looks for the socket
    /// (`crates/backend-http/src/lib.rs`).
    pub backend_socket: &'a Path,
}

/// A sandbox's cgroup under `demi-cloud`, named by the sandbox id, and its
/// limits.
pub struct Cgroup<'a> {
    pub name: &'a str,
    pub limits: Limits,
}

pub fn spec(boot: &Boot<'_>) -> Result<Spec, OciSpecError> {
    let none = HashSet::<Capability>::new;
    let capabilities = LinuxCapabilitiesBuilder::default()
        .bounding(BOUNDING.into_iter().collect::<HashSet<_>>())
        .effective(none())
        .permitted(none())
        .inheritable(none())
        .ambient(none())
        .build()?;
    let rlimit = |typ, limit| {
        PosixRlimitBuilder::default()
            .typ(typ)
            .hard(limit)
            .soft(limit)
            .build()
    };
    let process = ProcessBuilder::default()
        .terminal(false)
        // The runner and its jobs make files with the usual mask 022, not the
        // manager's own 077 (`crates/runner-shell/src/process_builtins.rs`).
        .user(
            UserBuilder::default()
                .uid(USER_ID)
                .gid(USER_ID)
                .umask(0o022u32)
                .build()?,
        )
        .cwd("/home/demi")
        .args(
            [
                INIT_PATH,
                "--",
                RUNNER_PATH,
                "run",
                "--managed-boot",
                BOOT_RECORD,
            ]
            .map(String::from)
            .to_vec(),
        )
        .env(vec![
            "HOME=/home/demi".into(),
            "USER=demi".into(),
            "LOGNAME=demi".into(),
            "LANG=C.UTF-8".into(),
            "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".into(),
            format!("{RELEASE_ENV}={}", boot.runner_release),
            // On the system image, not the 256 MiB `/tmp` in
            // memory.
            format!("TMPDIR={TEMPORARY_DIRECTORY}"),
        ])
        .capabilities(capabilities)
        .no_new_privileges(false)
        .rlimits(vec![
            rlimit(PosixRlimitType::RlimitNofile, 65536)?,
            rlimit(PosixRlimitType::RlimitNproc, PROCESS_LIMIT)?,
        ])
        .build()?;
    let namespace = |typ| LinuxNamespaceBuilder::default().typ(typ).build();
    let linux = LinuxBuilder::default()
        .namespaces(vec![
            namespace(LinuxNamespaceType::Pid)?,
            namespace(LinuxNamespaceType::Ipc)?,
            namespace(LinuxNamespaceType::Uts)?,
            namespace(LinuxNamespaceType::Mount)?,
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Network)
                .path(format!("/run/netns/{}", boot.namespace))
                .build()?,
        ])
        .masked_paths(
            ["/proc/kcore", "/proc/keys", "/proc/timer_list", "/sys"]
                .map(String::from)
                .to_vec(),
        )
        .readonly_paths(
            ["/proc/sys", "/proc/sysrq-trigger", "/proc/irq", "/proc/bus"]
                .map(String::from)
                .to_vec(),
        );
    let linux = match &boot.cgroup {
        Some(cgroup) => {
            let quota = i64::from(cgroup.limits.cpus.get()) * CPU_PERIOD as i64;
            let memory = i64::try_from(cgroup.limits.memory_bytes().get())
                .expect("a memory limit fits in i64");
            linux
                .cgroups_path(format!("/demi-cloud/{}", cgroup.name))
                .resources(
                    LinuxResourcesBuilder::default()
                        .cpu(
                            LinuxCpuBuilder::default()
                                .period(CPU_PERIOD)
                                .quota(quota)
                                .build()?,
                        )
                        // The same limit for memory and memory with swap: no swap.
                        .memory(
                            LinuxMemoryBuilder::default()
                                .limit(memory)
                                .swap(memory)
                                .build()?,
                        )
                        .pids(
                            LinuxPidsBuilder::default()
                                .limit(PROCESS_LIMIT as i64)
                                .build()?,
                        )
                        .build()?,
                )
        }
        None => linux,
    };
    let linux = linux.build()?;
    let mut spec = SpecBuilder::default()
        .version("1.1.0")
        .root(
            RootBuilder::default()
                .path(boot.directory.rootfs())
                .readonly(false)
                .build()?,
        )
        .process(process)
        .hostname("demi-cloud")
        .mounts(mounts(boot.directory, boot.backend_socket)?)
        .linux(linux)
        .build()?;
    // The builder starts from oci-spec's default, which carries an empty
    // annotation map; the workload gets no annotations at all.
    spec.set_annotations(None);
    Ok(spec)
}

fn mounts(directory: &RuntimeDirectory, backend_socket: &Path) -> Result<Vec<Mount>, OciSpecError> {
    let guest_socket = Path::new(BACKEND_SOCKET)
        .parent()
        .and_then(Path::to_str)
        .expect("the runner's socket lies in a directory");
    let mount = |destination: &str, typ: &str, source: &str, options: &[&str]| {
        MountBuilder::default()
            .destination(destination)
            .typ(typ)
            .source(source)
            .options(
                options
                    .iter()
                    .map(|option| (*option).to_owned())
                    .collect::<Vec<_>>(),
            )
            .build()
    };
    let bind = |source: std::path::PathBuf, destination: &str, options: &[&str]| {
        MountBuilder::default()
            .destination(destination)
            .typ("bind")
            .source(source)
            .options(
                options
                    .iter()
                    .map(|option| (*option).to_owned())
                    .collect::<Vec<_>>(),
            )
            .build()
    };
    Ok(vec![
        mount("/proc", "proc", "proc", &["nosuid", "nodev", "noexec"])?,
        mount(
            "/dev",
            "tmpfs",
            "tmpfs",
            &["nosuid", "mode=755", "size=65536k"],
        )?,
        mount(
            "/dev/pts",
            "devpts",
            "devpts",
            &[
                "nosuid",
                "noexec",
                "newinstance",
                "ptmxmode=0666",
                "mode=0620",
                "gid=5",
            ],
        )?,
        mount(
            "/dev/shm",
            "tmpfs",
            "tmpfs",
            &["nosuid", "nodev", "size=256m", "mode=1777"],
        )?,
        mount(
            "/tmp",
            "tmpfs",
            "tmpfs",
            &["nosuid", "nodev", "size=256m", "mode=1777"],
        )?,
        mount(
            "/run",
            "tmpfs",
            "tmpfs",
            &[
                "nosuid",
                "nodev",
                "size=256m",
                "mode=0755",
                "uid=1000",
                "gid=1000",
            ],
        )?,
        bind(directory.home(), "/home", &["bind", "nodev"])?,
        bind(directory.programs(), PROGRAMS_PATH, &["bind", "ro", "nodev"])?,
        bind(directory.boot(), BOOT_RECORD, &["bind", "ro", "nodev"])?,
        bind(
            backend_socket.to_owned(),
            guest_socket,
            &["bind", "ro", "nodev", "nosuid", "noexec"],
        )?,
        bind(
            directory.resolver(),
            "/etc/resolv.conf",
            &["bind", "ro", "nodev"],
        )?,
        bind(directory.hosts(), "/etc/hosts", &["bind", "ro", "nodev"])?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_is_the_shipped_profile() {
        let directory = RuntimeDirectory::new(
            std::path::Path::new("/run/demi-machine-manager"),
            "demi-00000000-0000-4000-8000-000000000000",
        );
        let spec = spec(&Boot {
            directory: &directory,
            namespace: "demi-3",
            cgroup: Some(Cgroup {
                name: "demi-00000000-0000-4000-8000-000000000000",
                limits: Limits {
                    cpus: std::num::NonZeroU32::new(2).unwrap(),
                    memory_mib: std::num::NonZeroU32::new(2048).unwrap(),
                    claude_code_memory_mib: std::num::NonZeroU32::new(4096).unwrap(),
                },
            }),
            runner_release: "release-of-the-runner",
            backend_socket: std::path::Path::new("/run/demi-backend"),
        })
        .unwrap();
        let expected: Spec =
            serde_json::from_str(include_str!("../../tests/fixtures/oci-config.json")).unwrap();
        assert_eq!(spec, expected);
    }
}
