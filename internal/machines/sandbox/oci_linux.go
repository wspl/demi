//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

// BootRecord is where the runner reads its boot record inside the sandbox.
const BootRecord = "/run/demi-boot.json"

// Limits specifies the nonzero CPU budget and memory limit configured by the manager.
// A nil *Limits disables cgroups; zero values do not select defaults.
type Limits struct {
	CPUs      uint32
	MemoryMiB uint32
}

// MemoryBytes returns the configured memory limit in bytes.
func (l Limits) MemoryBytes() uint64 { panic("not written: m-sandbox") }

// Boot contains the variable inputs to the fixed OCI profile.
type Boot struct {
	Directory RuntimeDirectory
	// Namespace is the network namespace's name under /run/netns.
	Namespace string
	// Cgroup is nil with resource limits off.
	Cgroup *Cgroup
}

// Cgroup names a sandbox's cgroup under demi-cloud and its resource limits.
type Cgroup struct {
	Name   ID
	Limits Limits
}

// Spec encodes the fixed OCI profile using typed, ordered fields. The returned
// JSON contains credential paths, never credentials. No caller may add flags,
// mounts, capabilities or annotations to the profile.
func Spec(boot Boot) ([]byte, error) { panic("not written: m-sandbox") }
