use super::{BootConfig, Network};
use crate::{mode::Options, volumes::BlockVolume};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs, io,
    os::unix::ffi::OsStrExt,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const UID: u32 = 1000;
const GID: u32 = 1000;
const HOME: &str = "/home/demi";
const STATE: &str = "/run/demi";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

pub struct Boot {
    pub options: Options,
}

/// Called before creating any runtime threads and only when this process is PID 1.
pub fn boot() -> io::Result<Boot> {
    if std::process::id() != 1 || unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "guest boot requires root PID 1",
        ));
    }
    mount(
        "proc",
        "/proc",
        "proc",
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )?;
    let config = BootConfig::parse(&fs::read_to_string("/proc/cmdline")?)?;
    mount(
        "sys",
        "/sys",
        "sysfs",
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )?;
    mount(
        "run",
        "/run",
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=0755"),
    )?;
    mount("", "/", "", libc::MS_REC | libc::MS_PRIVATE, None)?;
    fs::create_dir_all("/run/upper")?;
    fs::create_dir_all("/run/newroot")?;
    mount("/dev/vdc", "/run/upper", "ext4", 0, None)?;
    resize("/dev/vdc")?;
    fs::create_dir_all("/run/upper/upper")?;
    fs::create_dir_all("/run/upper/work")?;
    mount(
        "overlay",
        "/run/newroot",
        "overlay",
        0,
        Some("lowerdir=/,upperdir=/run/upper/upper,workdir=/run/upper/work"),
    )?;
    fs::create_dir_all("/run/newroot/oldroot")?;
    for path in ["/proc", "/sys", "/dev"] {
        let target = format!("/run/newroot{path}");
        fs::create_dir_all(&target)?;
        mount(path, &target, "", libc::MS_MOVE, None)?;
    }
    let root = CString::new("/run/newroot").unwrap();
    let old = CString::new("/run/newroot/oldroot").unwrap();
    if unsafe { libc::syscall(libc::SYS_pivot_root, root.as_ptr(), old.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    std::env::set_current_dir("/")?;
    mount(
        "run",
        "/run",
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=0755"),
    )?;
    fs::create_dir_all("/home")?;
    mount("/dev/vdb", "/home", "ext4", 0, None)?;
    resize("/dev/vdb")?;
    mount(
        "tmp",
        "/tmp",
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=1777"),
    )?;
    fs::create_dir_all("/dev/shm")?;
    mount(
        "shm",
        "/dev/shm",
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=1777"),
    )?;
    if unsafe { libc::sethostname(c"demi".as_ptr().cast(), 4) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if let Some(network) = config.network {
        configure_network(&network)?;
        let resolvers: String = network
            .dns
            .iter()
            .map(|address| format!("nameserver {address}\n"))
            .collect();
        fs::write("/etc/resolv.conf", resolvers)?;
    }
    fs::create_dir_all(STATE)?;
    fs::create_dir_all(HOME)?;
    chown(Path::new(STATE))?;
    if config.first_boot {
        copy_skeleton(Path::new("/etc/skel"), Path::new(HOME))?;
    }
    own_home(Path::new(HOME), config.first_boot)?;
    Ok(Boot {
        options: Options {
            backend: config.backend,
            directory: STATE.into(),
            executable: std::env::current_exe()?,
            cwd: HOME.into(),
            env: BTreeMap::from([
                ("PATH".into(), PATH.into()),
                ("HOME".into(), HOME.into()),
                ("USER".into(), "demi".into()),
                ("SHELL".into(), "/bin/bash".into()),
                ("LANG".into(), "C".into()),
            ]),
            runner: crate::connection::wire::HelloRunner {
                native_target: Some(crate::commands::native::target().into()),
                name: "demi".into(),
                platform: "linux".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                managed: Some(true),
                identity: crate::connection::wire::HelloRunnerIdentity {
                    uid: UID.into(),
                    gid: GID.into(),
                    hostname: "demi".into(),
                    home_dir: HOME.into(),
                },
            },
            token: Some(config.token),
            volumes: vec![
                BlockVolume {
                    name: "home".into(),
                    device: "/dev/vdb".into(),
                    mount: "/home".into(),
                },
                BlockVolume {
                    name: "system".into(),
                    device: "/dev/vdc".into(),
                    mount: "/oldroot/run/upper".into(),
                },
            ],
        },
    })
}

fn mount(
    source: &str,
    target: &str,
    filesystem: &str,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let source = CString::new(source).map_err(io::Error::other)?;
    let target = CString::new(target).map_err(io::Error::other)?;
    let filesystem = CString::new(filesystem).map_err(io::Error::other)?;
    let data = data
        .map(CString::new)
        .transpose()
        .map_err(io::Error::other)?;
    if unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            flags,
            data.as_ref()
                .map_or(std::ptr::null(), |data| data.as_ptr().cast()),
        )
    } != 0
    {
        return Err(io::Error::other(format!(
            "mount {}: {}",
            target.to_string_lossy(),
            io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn resize(device: &str) -> io::Result<()> {
    let status = Command::new("resize2fs")
        .arg(device)
        .env_clear()
        .env("PATH", PATH)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("resize2fs {device} failed")))
    }
}

fn chown(path: &Path) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    if unsafe { libc::lchown(path.as_ptr(), UID, GID) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn own_home(path: &Path, recursive: bool) -> io::Result<()> {
    chown(path)?;
    if recursive && fs::symlink_metadata(path)?.is_dir() {
        for child in fs::read_dir(path)? {
            own_home(&child?.path(), true)?;
        }
    }
    Ok(())
}
fn copy_skeleton(source: &Path, target: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from)?;
        if metadata.is_dir() {
            fs::create_dir_all(&to)?;
            copy_skeleton(&from, &to)?;
            fs::set_permissions(&to, metadata.permissions())?;
        } else if metadata.file_type().is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
        } else if metadata.is_file() {
            fs::copy(from, &to)?;
            fs::set_permissions(&to, metadata.permissions())?;
        } else {
            return Err(io::Error::other("unsupported file in guest skeleton"));
        }
    }
    Ok(())
}

fn socket_address(address: std::net::Ipv4Addr) -> libc::sockaddr {
    let mut value: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    value.sin_family = libc::AF_INET as _;
    value.sin_addr.s_addr = u32::from_ne_bytes(address.octets());
    unsafe { std::mem::transmute(value) }
}
fn interface(name: &str) -> io::Result<libc::ifreq> {
    if name.len() >= libc::IFNAMSIZ {
        return Err(io::Error::other("network interface name is too long"));
    }
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (to, from) in request.ifr_name.iter_mut().zip(name.bytes()) {
        *to = from as _;
    }
    Ok(request)
}
fn configure_network(network: &Network) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    for name in ["lo", "eth0"] {
        let mut request = interface(name)?;
        if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) } != 0 {
            return Err(io::Error::last_os_error());
        }
        request.ifr_ifru.ifru_flags = unsafe { request.ifr_ifru.ifru_flags } | libc::IFF_UP as i16;
        if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCSIFFLAGS as _, &request) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut address = interface("eth0")?;
    address.ifr_ifru.ifru_addr = socket_address(network.address);
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCSIFADDR as _, &address) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mask = if network.prefix == 0 {
        0
    } else {
        u32::MAX << (32 - network.prefix)
    };
    address.ifr_ifru.ifru_netmask = socket_address(mask.into());
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCSIFNETMASK as _, &address) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut route: libc::rtentry = unsafe { std::mem::zeroed() };
    route.rt_dst = socket_address(std::net::Ipv4Addr::UNSPECIFIED);
    route.rt_genmask = socket_address(std::net::Ipv4Addr::UNSPECIFIED);
    route.rt_gateway = socket_address(network.gateway);
    route.rt_flags = (libc::RTF_UP | libc::RTF_GATEWAY) as _;
    let device = CString::new("eth0").unwrap();
    route.rt_dev = device.as_ptr().cast_mut();
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCADDRT as _, &route) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// The parent alone reaps adopted orphans; it cannot steal runner-owned child exits.
/// Returns None in the unprivileged runner child, Some(exit code) in PID 1.
pub fn supervise() -> io::Result<Option<u8>> {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        if unsafe { libc::setgroups(0, std::ptr::null()) } != 0
            || unsafe { libc::setgid(GID) } != 0
            || unsafe { libc::setuid(UID) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        std::env::set_current_dir(HOME)?;
        return Ok(None);
    }
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = stop as *const () as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut stopping = None;
    let code = loop {
        if STOP.load(Ordering::Relaxed) && stopping.is_none() {
            if unsafe { libc::kill(pid, libc::SIGTERM) } != 0
                && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                return Err(io::Error::last_os_error());
            }
            stopping = Some(Instant::now());
        }
        if stopping.is_some_and(|start| start.elapsed() >= Duration::from_secs(10)) {
            unsafe {
                libc::kill(-1, libc::SIGKILL);
            }
        }
        let mut status = 0;
        let exited = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if exited == pid {
            break if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status) as u8
            } else {
                1
            };
        }
        if exited < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        }
        if exited == 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // PID 1's exit ends the guest; stop and reap surviving descendants first.
    unsafe {
        libc::kill(-1, libc::SIGKILL);
    }
    loop {
        let mut status = 0;
        if unsafe { libc::waitpid(-1, &mut status, 0) } >= 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ECHILD) {
            break;
        }
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    unsafe {
        libc::sync();
    }
    Ok(Some(code))
}
