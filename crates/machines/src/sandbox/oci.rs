//! One boot's OCI runtime configuration (`managed-hosts.md` § Isolation and
//! joining): the prepared root, UID 1000's process under init, the runtime
//! mounts, the network namespace and the cgroup limits. The profile is code,
//! not configuration: only the paths, the slot and the budget vary.

use std::{
    collections::HashSet,
    num::{NonZeroU32, NonZeroU64},
};

use demi_machines_protocol::image::{INIT_PATH, RUNNER_PATH};
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
    /// The cgroup under `demi-cloud`, named by the sandbox id.
    pub cgroup: &'a str,
    /// The network namespace's name under `/run/netns`.
    pub namespace: &'a str,
    pub cpus: NonZeroU32,
    pub memory_bytes: NonZeroU64,
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
        .user(UserBuilder::default().uid(USER_ID).gid(USER_ID).build()?)
        .cwd("/home/demi")
        .args(
            [INIT_PATH, "--", RUNNER_PATH, "run", "--managed-boot", BOOT_RECORD]
                .map(String::from)
                .to_vec(),
        )
        .env(
            [
                "HOME=/home/demi",
                "USER=demi",
                "LOGNAME=demi",
                "LANG=en_US.UTF-8",
                "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin",
            ]
            .map(String::from)
            .to_vec(),
        )
        .capabilities(capabilities)
        .no_new_privileges(false)
        .rlimits(vec![
            rlimit(PosixRlimitType::RlimitNofile, 65536)?,
            rlimit(PosixRlimitType::RlimitNproc, PROCESS_LIMIT)?,
        ])
        .build()?;
    let namespace = |typ| LinuxNamespaceBuilder::default().typ(typ).build();
    let quota = i64::from(boot.cpus.get()) * CPU_PERIOD as i64;
    let memory = i64::try_from(boot.memory_bytes.get()).expect("a memory limit fits in i64");
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
        .cgroups_path(format!("/demi-cloud/{}", boot.cgroup))
        .resources(
            LinuxResourcesBuilder::default()
                .cpu(LinuxCpuBuilder::default().period(CPU_PERIOD).quota(quota).build()?)
                // The same limit for memory and memory with swap: no swap.
                .memory(LinuxMemoryBuilder::default().limit(memory).swap(memory).build()?)
                .pids(LinuxPidsBuilder::default().limit(PROCESS_LIMIT as i64).build()?)
                .build()?,
        )
        .masked_paths(
            ["/proc/kcore", "/proc/keys", "/proc/timer_list", "/sys"]
                .map(String::from)
                .to_vec(),
        )
        .readonly_paths(
            ["/proc/sys", "/proc/sysrq-trigger", "/proc/irq", "/proc/bus"]
                .map(String::from)
                .to_vec(),
        )
        .build()?;
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
        .mounts(mounts(boot.directory)?)
        .linux(linux)
        .build()?;
    // The builder starts from oci-spec's default, which carries an empty
    // annotation map; the workload gets no annotations at all.
    spec.set_annotations(None);
    Ok(spec)
}

fn mounts(directory: &RuntimeDirectory) -> Result<Vec<Mount>, OciSpecError> {
    let mount = |destination: &str, typ: &str, source: &str, options: &[&str]| {
        MountBuilder::default()
            .destination(destination)
            .typ(typ)
            .source(source)
            .options(options.iter().map(|option| (*option).to_owned()).collect::<Vec<_>>())
            .build()
    };
    let bind = |source: std::path::PathBuf, destination: &str, options: &[&str]| {
        MountBuilder::default()
            .destination(destination)
            .typ("bind")
            .source(source)
            .options(options.iter().map(|option| (*option).to_owned()).collect::<Vec<_>>())
            .build()
    };
    Ok(vec![
        mount("/proc", "proc", "proc", &["nosuid", "nodev", "noexec"])?,
        mount("/dev", "tmpfs", "tmpfs", &["nosuid", "mode=755", "size=65536k"])?,
        mount(
            "/dev/pts",
            "devpts",
            "devpts",
            &["nosuid", "noexec", "newinstance", "ptmxmode=0666", "mode=0620", "gid=5"],
        )?,
        mount("/dev/shm", "tmpfs", "tmpfs", &["nosuid", "nodev", "size=256m", "mode=1777"])?,
        mount("/tmp", "tmpfs", "tmpfs", &["nosuid", "nodev", "size=256m", "mode=1777"])?,
        mount(
            "/run",
            "tmpfs",
            "tmpfs",
            &["nosuid", "nodev", "size=256m", "mode=0755", "uid=1000", "gid=1000"],
        )?,
        bind(directory.home(), "/home", &["bind", "nodev"])?,
        bind(directory.boot(), BOOT_RECORD, &["bind", "ro", "nodev"])?,
        bind(directory.resolver(), "/etc/resolv.conf", &["bind", "ro", "nodev"])?,
        bind(directory.hosts(), "/etc/hosts", &["bind", "ro", "nodev"])?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_is_the_shipped_profile() {
        let directory = RuntimeDirectory::new(
            std::path::Path::new("/run/demi-machines"),
            "demi-00000000-0000-4000-8000-000000000000",
        );
        let spec = spec(&Boot {
            directory: &directory,
            cgroup: "demi-00000000-0000-4000-8000-000000000000",
            namespace: "demi-3",
            cpus: NonZeroU32::new(2).unwrap(),
            memory_bytes: NonZeroU64::new(2048 << 20).unwrap(),
        })
        .unwrap();
        let expected: Spec =
            serde_json::from_str(include_str!("../../tests/fixtures/oci-config.json")).unwrap();
        assert_eq!(spec, expected);
    }
}
