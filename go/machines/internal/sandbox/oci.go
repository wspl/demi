package sandbox

import (
	"encoding/json/jsontext"
	"encoding/json/v2"

	"github.com/opencontainers/runtime-spec/specs-go"

	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machinesproto"
)

// One boot's OCI runtime configuration (docs/cloud/managed-hosts.md § Isolation
// and joining): the prepared root, UID 1000's process under init, the runtime
// mounts, the network namespace and, with the resource limits on, the cgroup and
// its limits. The profile is code, not configuration: only the paths, the slot
// and the limits vary.

// bounding are the capabilities setuid programs such as sudo may gain; the
// process itself starts with none.
var bounding = []string{
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
}

// BootRecord is where the runner reads its boot record inside the sandbox.
const BootRecord = "/run/demi-boot.json"

// cpuPeriod is the CFS period the CPU budget is a quota of.
const cpuPeriod = 100_000

// processLimit is the sandbox's process limit, host PIDs and ordinary-user
// processes alike.
const processLimit = 1024

// A Boot is what one boot's configuration varies by.
type Boot struct {
	Directory *RuntimeDirectory
	// Namespace is the network namespace's name under /run/netns.
	Namespace string
	// Cgroup is the boot's cgroup; nil with the resource limits off, when the
	// sandbox has none.
	Cgroup *CgroupLimits
}

// CgroupLimits are a sandbox's cgroup under demi-cloud, named by the sandbox id,
// and its limits.
type CgroupLimits struct {
	Name   string
	Limits config.Limits
}

// Spec returns the boot's OCI runtime configuration.
func Spec(boot Boot) *specs.Spec {
	umask := uint32(0o022)
	rlimit := func(kind string, limit uint64) specs.POSIXRlimit {
		return specs.POSIXRlimit{Type: kind, Hard: limit, Soft: limit}
	}
	process := &specs.Process{
		Terminal: false,
		// The runner and its jobs make files with the usual mask 022, not the
		// manager's own 077 (docs/execution/runner.md § Builtins that act on a
		// process).
		User: specs.User{UID: UserID, GID: UserID, Umask: &umask},
		Cwd:  "/home/demi",
		Args: []string{machinesproto.InitPath, "--", machinesproto.RunnerPath, "run", "--managed-boot", BootRecord},
		Env: []string{
			"HOME=/home/demi",
			"USER=demi",
			"LOGNAME=demi",
			"LANG=en_US.UTF-8",
			"PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin",
		},
		Capabilities: &specs.LinuxCapabilities{
			Bounding:    bounding,
			Effective:   []string{},
			Permitted:   []string{},
			Inheritable: []string{},
			Ambient:     []string{},
		},
		NoNewPrivileges: false,
		Rlimits: []specs.POSIXRlimit{
			rlimit("RLIMIT_NOFILE", 65536),
			rlimit("RLIMIT_NPROC", processLimit),
		},
	}
	linux := &specs.Linux{
		Namespaces: []specs.LinuxNamespace{
			{Type: specs.PIDNamespace},
			{Type: specs.IPCNamespace},
			{Type: specs.UTSNamespace},
			{Type: specs.MountNamespace},
			{Type: specs.NetworkNamespace, Path: "/run/netns/" + boot.Namespace},
		},
		MaskedPaths:   []string{"/proc/kcore", "/proc/keys", "/proc/timer_list", "/sys"},
		ReadonlyPaths: []string{"/proc/sys", "/proc/sysrq-trigger", "/proc/irq", "/proc/bus"},
	}
	if cgroup := boot.Cgroup; cgroup != nil {
		quota := int64(cgroup.Limits.CPUs) * cpuPeriod
		period := uint64(cpuPeriod)
		// The same limit for memory and memory with swap: no swap.
		memory := int64(cgroup.Limits.MemoryBytes())
		pids := int64(processLimit)
		linux.CgroupsPath = "/demi-cloud/" + cgroup.Name
		linux.Resources = &specs.LinuxResources{
			CPU:    &specs.LinuxCPU{Period: &period, Quota: &quota},
			Memory: &specs.LinuxMemory{Limit: &memory, Swap: &memory},
			Pids:   &specs.LinuxPids{Limit: &pids},
		}
	}
	return &specs.Spec{
		Version:  "1.1.0",
		Root:     &specs.Root{Path: boot.Directory.RootFS(), Readonly: false},
		Process:  process,
		Hostname: "demi-cloud",
		Mounts:   mounts(boot.Directory),
		Linux:    linux,
	}
}

func mounts(directory *RuntimeDirectory) []specs.Mount {
	mount := func(destination, kind, source string, options ...string) specs.Mount {
		return specs.Mount{Destination: destination, Type: kind, Source: source, Options: options}
	}
	return []specs.Mount{
		mount("/proc", "proc", "proc", "nosuid", "nodev", "noexec"),
		mount("/dev", "tmpfs", "tmpfs", "nosuid", "mode=755", "size=65536k"),
		mount("/dev/pts", "devpts", "devpts", "nosuid", "noexec", "newinstance", "ptmxmode=0666", "mode=0620", "gid=5"),
		mount("/dev/shm", "tmpfs", "tmpfs", "nosuid", "nodev", "size=256m", "mode=1777"),
		mount("/tmp", "tmpfs", "tmpfs", "nosuid", "nodev", "size=256m", "mode=1777"),
		mount("/run", "tmpfs", "tmpfs", "nosuid", "nodev", "size=256m", "mode=0755", "uid=1000", "gid=1000"),
		mount("/home", "bind", directory.Home(), "bind", "nodev"),
		mount(BootRecord, "bind", directory.Boot(), "bind", "ro", "nodev"),
		mount("/etc/resolv.conf", "bind", directory.Resolver(), "bind", "ro", "nodev"),
		mount("/etc/hosts", "bind", directory.Hosts(), "bind", "ro", "nodev"),
	}
}

// capabilitySets are the five sets of a process, each written even when it is
// empty: an absent set may mean the runtime's default, an empty one means none.
// The fields are those of specs.LinuxCapabilities, whose tags leave an empty set
// out.
type capabilitySets struct {
	Bounding    []string `json:"bounding"`
	Effective   []string `json:"effective"`
	Inheritable []string `json:"inheritable"`
	Permitted   []string `json:"permitted"`
	Ambient     []string `json:"ambient"`
}

// Config returns the JSON of spec, the file the runtime reads, with every
// capability set written.
func Config(spec *specs.Spec) ([]byte, error) {
	writeSets := json.MarshalToFunc(func(enc *jsontext.Encoder, sets specs.LinuxCapabilities) error {
		return json.MarshalEncode(enc, capabilitySets(sets))
	})
	return json.Marshal(spec, json.WithMarshalers(writeSets))
}
