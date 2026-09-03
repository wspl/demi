//! `demi-fc-helper` (`docs/demi-next/managed-hosts.md` § Provisioning): the
//! only root in a `jailer`-mode deployment. Two verbs, every argument
//! whitelisted, no shell, no configuration file.
//!
//!   vm start --id ID --jailer PATH --firecracker PATH --chroot-base DIR --uid N --gid N
//!            --backend-gid N --kernel PATH --rootfs PATH --home PATH
//!       prepares `<chroot-base>/<firecracker basename>/<id>/root` (the kernel and
//!       rootfs hardlinked or copied in, the home image hardlinked and made
//!       writable by the VM uid and the backend group, the API socket directory
//!       group-accessible), writes the child's pid beside it, execs the jailer
//!       as a child and waits: the helper's exit is the VM's exit.
//!   vm kill --id ID --chroot-base DIR
//!       SIGKILLs the pid recorded by `vm start`.
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::{chown, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

fn fail(message: &str) -> ! {
    eprintln!("demi-fc-helper: {message}");
    exit(2)
}

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        if !key.starts_with("--") || i + 1 >= args.len() {
            fail(&format!("bad argument {key}"));
        }
        flags.insert(key.trim_start_matches("--").to_string(), args[i + 1].clone());
        i += 2;
    }
    flags
}

fn required<'a>(flags: &'a HashMap<String, String>, name: &str) -> &'a str {
    flags.get(name).map(String::as_str).unwrap_or_else(|| fail(&format!("--{name} is required")))
}

fn valid_id(id: &str) -> &str {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        fail("--id must be [A-Za-z0-9_-]{1,64}");
    }
    id
}

fn absolute(flags: &HashMap<String, String>, name: &str) -> PathBuf {
    let value = required(flags, name);
    let path = Path::new(value);
    if !path.is_absolute() || value.contains("/../") || value.ends_with("/..") {
        fail(&format!("--{name} must be an absolute path without .."));
    }
    path.to_path_buf()
}

fn number(flags: &HashMap<String, String>, name: &str, min: u32) -> u32 {
    let value: u32 = required(flags, name).parse().unwrap_or_else(|_| fail(&format!("--{name} must be a number")));
    if value < min {
        fail(&format!("--{name} must be at least {min}"));
    }
    value
}

/// A hardlink where the filesystem allows one, a copy otherwise.
fn link_or_copy(from: &Path, to: &Path) {
    let _ = fs::remove_file(to);
    if fs::hard_link(from, to).is_err() {
        fs::copy(from, to).unwrap_or_else(|e| fail(&format!("copy {}: {e}", from.display())));
    }
}

fn start(flags: HashMap<String, String>) -> ! {
    let id = valid_id(required(&flags, "id"));
    let jailer = absolute(&flags, "jailer");
    let firecracker = absolute(&flags, "firecracker");
    let chroot_base = absolute(&flags, "chroot-base");
    let uid = number(&flags, "uid", 1000);
    let gid = number(&flags, "gid", 1000);
    let backend_gid = number(&flags, "backend-gid", 1);
    let kernel = absolute(&flags, "kernel");
    let rootfs = absolute(&flags, "rootfs");
    let home = absolute(&flags, "home");
    let exec_name = firecracker.file_name().unwrap_or_else(|| fail("--firecracker has no file name")).to_owned();
    let jail = chroot_base.join(&exec_name).join(id);
    let root = jail.join("root");
    let _ = fs::remove_dir_all(&jail);
    fs::create_dir_all(root.join("run")).unwrap_or_else(|e| fail(&format!("mkdir {}: {e}", root.display())));
    link_or_copy(&kernel, &root.join("vmlinux"));
    link_or_copy(&rootfs, &root.join("rootfs.ext4"));
    // The working image is shared with the backend by the link: the VM uid owns it, the backend group writes it.
    let home_link = root.join("home.ext4");
    let _ = fs::remove_file(&home_link);
    fs::hard_link(&home, &home_link).unwrap_or_else(|e| fail(&format!("link {} into the jail (same filesystem required): {e}", home.display())));
    chown(&home_link, Some(uid), Some(backend_gid)).unwrap_or_else(|e| fail(&format!("chown home: {e}")));
    fs::set_permissions(&home_link, fs::Permissions::from_mode(0o660)).unwrap_or_else(|e| fail(&format!("chmod home: {e}")));
    // The API socket lands in run/ with the backend group, through the setgid bit and the umask below.
    let run = root.join("run");
    chown(&run, Some(uid), Some(backend_gid)).unwrap_or_else(|e| fail(&format!("chown run: {e}")));
    fs::set_permissions(&run, fs::Permissions::from_mode(0o2770)).unwrap_or_else(|e| fail(&format!("chmod run: {e}")));
    chown(&root, Some(uid), Some(gid)).unwrap_or_else(|e| fail(&format!("chown root: {e}")));

    let mut command = Command::new(&jailer);
    command
        .arg("--id").arg(id)
        .arg("--exec-file").arg(&firecracker)
        .arg("--uid").arg(uid.to_string())
        .arg("--gid").arg(gid.to_string())
        .arg("--chroot-base-dir").arg(&chroot_base)
        .arg("--cgroup-version").arg("2")
        .arg("--new-pid-ns")
        .arg("--")
        .arg("--api-sock").arg("/run/firecracker.socket")
        .arg("--id").arg(id);
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o007);
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap_or_else(|e| fail(&format!("spawn jailer: {e}")));
    fs::write(jail.join("pid"), child.id().to_string()).unwrap_or_else(|e| fail(&format!("write pid: {e}")));
    let status = child.wait().unwrap_or_else(|e| fail(&format!("wait: {e}")));
    let _ = fs::remove_dir_all(&jail);
    exit(status.code().unwrap_or(128))
}

fn kill(flags: HashMap<String, String>) -> ! {
    let id = valid_id(required(&flags, "id"));
    let chroot_base = absolute(&flags, "chroot-base");
    // The jail directory is named by the exec file; find this id under any exec name.
    let mut killed = false;
    if let Ok(entries) = fs::read_dir(&chroot_base) {
        for entry in entries.flatten() {
            let pid_file = entry.path().join(id).join("pid");
            if let Ok(text) = fs::read_to_string(&pid_file) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    if pid > 1 {
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                        killed = true;
                    }
                }
            }
        }
    }
    exit(if killed { 0 } else { 1 })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1).map(String::as_str)) {
        (Some("vm"), Some("start")) => start(parse_flags(&args[2..])),
        (Some("vm"), Some("kill")) => kill(parse_flags(&args[2..])),
        _ => fail("usage: demi-fc-helper vm start … | vm kill …"),
    }
}
