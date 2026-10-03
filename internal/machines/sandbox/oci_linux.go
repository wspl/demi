//go:build linux

package sandbox

import (
	"fmt"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
)

// BootRecord is where the runner reads its boot record inside the sandbox.
const BootRecord = "/run/demi-boot.json"

// Limits specifies the nonzero CPU budget and memory limit configured by the manager.
// A nil *Limits disables cgroups; zero values do not select defaults.
type Limits struct {
	// CPUs is the sandbox CPU budget.
	CPUs uint32
	// MemoryMiB is the sandbox memory budget in MiB.
	MemoryMiB uint32
}

// MemoryBytes returns the configured memory limit in bytes.
func (l Limits) MemoryBytes() uint64 {
	return uint64(l.MemoryMiB) * 1024 * 1024
}

// Boot contains the variable inputs to the fixed OCI profile.
type Boot struct {
	// Directory locates the runtime bundle.
	Directory RuntimeDirectory
	// Namespace is the network namespace's name under /run/netns.
	Namespace string
	// Cgroup is nil with resource limits off.
	Cgroup *Cgroup
}

// Cgroup names a sandbox's cgroup under demi-cloud and its resource limits.
type Cgroup struct {
	// Name identifies the boot whose cgroup is configured.
	Name ID
	// Limits contains this cgroup's resource budgets.
	Limits Limits
}

// Spec encodes the fixed OCI profile using typed, ordered fields. The returned
// JSON contains credential paths, never credentials. No caller may add flags,
// mounts, capabilities or annotations to the profile.
func Spec(boot Boot) ([]byte, error) {
	directory := boot.Directory
	namespace := "/run/netns/" + boot.Namespace
	spec := ociSpec{
		Version:  "1.1.0",
		Root:     ociRoot{Path: directory.RootFS()},
		Hostname: "demi-cloud",
		Process:  bootProcess(),
		Mounts:   bootMounts(directory),
		Linux: ociLinux{
			Namespaces: []ociNamespace{
				{Type: "pid"},
				{Type: "ipc"},
				{Type: "uts"},
				{Type: "mount"},
				{Type: "network", Path: &namespace},
			},
			MaskedPaths:   []string{"/proc/kcore", "/proc/keys", "/proc/timer_list", "/sys"},
			ReadonlyPaths: []string{"/proc/sys", "/proc/sysrq-trigger", "/proc/irq", "/proc/bus"},
		},
	}
	if group := boot.Cgroup; group != nil {
		// Rust's NonZeroU32 inputs make these values impossible there.
		if group.Limits.CPUs == 0 || group.Limits.MemoryMiB == 0 {
			return nil, fmt.Errorf("sandbox resource limits must be nonzero")
		}
		path := "/demi-cloud/" + string(group.Name)
		spec.Linux.CgroupsPath = &path
		memory := int64(group.Limits.MemoryBytes())
		spec.Linux.Resources = &ociResources{
			Memory: ociMemory{Limit: memory, Swap: memory},
			CPU:    ociCPU{Quota: int64(group.Limits.CPUs) * 100000, Period: 100000},
			Pids:   ociPids{Limit: 1024},
		}
	}
	return spec.MarshalJSON()
}

func bootProcess() ociProcess {
	return ociProcess{
		User: ociUser{UID: system.UserID, GID: system.UserID, Umask: 0o022},
		Cwd:  "/home/demi",
		Args: []string{machinewire.InitPath, "--", machinewire.RunnerPath, "run", "--managed-boot", BootRecord},
		Env: []string{
			"HOME=/home/demi",
			"USER=demi",
			"LOGNAME=demi",
			"LANG=en_US.UTF-8",
			"PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin",
		},
		Capabilities: ociCapabilities{
			Bounding: []string{
				"CAP_CHOWN",
				"CAP_DAC_OVERRIDE",
				"CAP_FOWNER",
				"CAP_FSETID",
				"CAP_KILL",
				"CAP_SETGID",
				"CAP_SETUID",
				"CAP_SETPCAP",
				"CAP_NET_BIND_SERVICE",
				"CAP_SYS_CHROOT",
				"CAP_SETFCAP",
			},
			Effective:   []string{},
			Inheritable: []string{},
			Permitted:   []string{},
			Ambient:     []string{},
		},
		Rlimits: []ociRlimit{
			{Type: "RLIMIT_NOFILE", Hard: 65536, Soft: 65536},
			{Type: "RLIMIT_NPROC", Hard: 1024, Soft: 1024},
		},
	}
}

func bootMounts(directory RuntimeDirectory) []ociMount {
	return []ociMount{
		{Destination: "/proc", Type: "proc", Source: "proc", Options: []string{"nosuid", "nodev", "noexec"}},
		{
			Destination: "/dev",
			Type:        "tmpfs",
			Source:      "tmpfs",
			Options:     []string{"nosuid", "mode=755", "size=65536k"},
		},
		{
			Destination: "/dev/pts",
			Type:        "devpts",
			Source:      "devpts",
			Options:     []string{"nosuid", "noexec", "newinstance", "ptmxmode=0666", "mode=0620", "gid=5"},
		},
		{
			Destination: "/dev/shm",
			Type:        "tmpfs",
			Source:      "tmpfs",
			Options:     []string{"nosuid", "nodev", "size=256m", "mode=1777"},
		},
		{
			Destination: "/tmp",
			Type:        "tmpfs",
			Source:      "tmpfs",
			Options:     []string{"nosuid", "nodev", "size=256m", "mode=1777"},
		},
		{
			Destination: "/run",
			Type:        "tmpfs",
			Source:      "tmpfs",
			Options:     []string{"nosuid", "nodev", "size=256m", "mode=0755", "uid=1000", "gid=1000"},
		},
		{Destination: "/home", Type: "bind", Source: directory.Home(), Options: []string{"bind", "nodev"}},
		{Destination: BootRecord, Type: "bind", Source: directory.Boot(), Options: []string{"bind", "ro", "nodev"}},
		{
			Destination: "/etc/resolv.conf",
			Type:        "bind",
			Source:      directory.Resolver(),
			Options:     []string{"bind", "ro", "nodev"},
		},
		{
			Destination: "/etc/hosts",
			Type:        "bind",
			Source:      directory.Hosts(),
			Options:     []string{"bind", "ro", "nodev"},
		},
	}
}
