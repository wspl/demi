package sandbox

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

// Cgroups (docs/cloud/managed-hosts.md § Resource limits): with the resource
// limits on, every sandbox's Sentry and Gofer live in demi-cloud/<sandbox> under
// the cgroup v2 root, which limits them together and lets the manager kill all of
// them. With the limits off, nothing here runs.

// DefaultCgroupRoot is the cgroup v2 root.
const DefaultCgroupRoot = "/sys/fs/cgroup"

var controllers = [...]string{"cpu", "memory", "pids"}

const (
	// fenceDeadline is how long writers get to exit once killed.
	fenceDeadline = 5 * time.Second
	fencePoll     = 50 * time.Millisecond
)

// A MissingControllersError means the host lacks controllers the limits need.
type MissingControllersError struct {
	Root  string
	Names []string
}

func (e *MissingControllersError) Error() string {
	return fmt.Sprintf("Cloud resource limits need the cgroup v2 cpu, memory and pids controllers at %s; missing: %s. DEMI_MANAGED_LIMITS=off runs Clouds without limits",
		e.Root, strings.Join(e.Names, ", "))
}

// ErrWriters means a sandbox's processes did not end after the kill.
var ErrWriters = errors.New("Cloud runtime writers did not terminate")

// Cgroups is the cgroup v2 hierarchy the sandboxes' cgroups live in.
type Cgroups struct {
	root string
}

// NewCgroups returns the cgroup hierarchy at root.
func NewCgroups(root string) *Cgroups {
	return &Cgroups{root: root}
}

func (c *Cgroups) sandboxes() string {
	return filepath.Join(c.root, "demi-cloud")
}

// Prepare requires the CPU, memory and PID controllers and enables them for the
// sandboxes' cgroups. Nothing under the root changes before all three are there;
// an error names every one that is missing.
func (c *Cgroups) Prepare() error {
	available, err := os.ReadFile(filepath.Join(c.root, "cgroup.controllers"))
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	// A root that is no cgroup v2 hierarchy offers no controller.
	offered := strings.Fields(string(available))
	var missing []string
	for _, controller := range controllers {
		if !slices.Contains(offered, controller) {
			missing = append(missing, controller)
		}
	}
	if len(missing) > 0 {
		return &MissingControllersError{Root: c.root, Names: missing}
	}
	const enable = "+cpu +memory +pids"
	if err := os.WriteFile(filepath.Join(c.root, "cgroup.subtree_control"), []byte(enable), 0o666); err != nil {
		return err
	}
	if err := os.MkdirAll(c.sandboxes(), 0o777); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(c.sandboxes(), "cgroup.subtree_control"), []byte(enable), 0o666)
}

// Fence kills whatever runs in sandbox's cgroup, waits for it to empty and
// removes it: a partially created runtime is fenced even when runsc never
// recorded it.
func (c *Cgroups) Fence(ctx context.Context, sandbox SandboxID) error {
	cgroup := filepath.Join(c.sandboxes(), string(sandbox))
	err := os.WriteFile(filepath.Join(cgroup, "cgroup.kill"), []byte("1"), 0o666)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	deadline := time.Now().Add(fenceDeadline)
	ticker := time.NewTicker(fencePoll)
	defer ticker.Stop()
	for {
		events, err := os.ReadFile(filepath.Join(cgroup, "cgroup.events"))
		if err != nil {
			return err
		}
		if slices.Contains(strings.Split(string(events), "\n"), "populated 0") {
			break
		}
		if !time.Now().Before(deadline) {
			return ErrWriters
		}
		select {
		case <-ticker.C:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return os.Remove(cgroup)
}
