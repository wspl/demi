//go:build linux

package sandbox

import (
	"context"
	_ "embed"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/machines/system"
)

// ProfileFlags returns the fixed systrap, sandbox network, shared filesystem,
// setuid and Directfs profile. Each call returns an independent array.
func ProfileFlags() [7]string {
	return [7]string{"--platform=systrap", "--network=sandbox", "--overlay2=none", "--file-access=shared", "--file-access-mounts=shared", "--allow-suid=true", "--directfs=true"}
}

// PinnedRelease returns the runtime release built into the manager.
func PinnedRelease() RuntimeRelease {
	// The embedded manifest is validated by the fixture test, not external input.
	release, _ := DecodeRuntimeRelease(releaseManifest)
	return release
}

// Version returns the pinned version for this architecture, including the
// seccomp trap fix on arm64.
func (r RuntimeRelease) Version() string {
	if runtime.GOARCH == "arm64" {
		return r.Arm64Version
	}
	return "release-" + r.Upstream
}

// ReportsVersion checks whether the first output line reports exactly version.
func ReportsVersion(output, version string) bool {
	line, _, terminated := strings.Cut(output, "\n")
	// Rust str::lines strips CR only as part of a CRLF line ending.
	if terminated {
		line = strings.TrimSuffix(line, "\r")
	}
	return line == "runsc version "+version
}

// StatusIn reads runsc list output. found is false for an absent container,
// including a null list; unknown fields in container entries are ignored.
func StatusIn(listing []byte, id ID) (status Status, found bool, err error) {
	if contract.IsNull(listing) {
		return "", false, nil
	}
	containers, err := decodeRuntimeListing(listing)
	if err != nil {
		return "", false, err
	}
	for _, container := range containers {
		if container.ID == string(id) {
			return container.Status, true, nil
		}
	}
	return "", false, nil
}

// Runsc drives the pinned runtime with state under the manager's runtime directory.
type Runsc struct {
	tools   *system.Tools
	root    string
	cgroups bool
}

// NewRunsc selects the resolved tools, runtime root and explicit cgroup mode.
func NewRunsc(tools *system.Tools, runtime string, cgroups bool) *Runsc {
	return &Runsc{tools: tools, root: filepath.Join(runtime, "runsc"), cgroups: cgroups}
}

// Root returns where runsc keeps its containers' state.
func (r *Runsc) Root() string {
	return r.root
}

// Args prepends the state root and fixed profile, including --ignore-cgroups
// when resource limits are off.
func (r *Runsc) Args(command []string) []string {
	args := []string{"--root=" + r.root}
	profile := ProfileFlags()
	args = append(args, profile[:]...)
	if !r.cgroups {
		args = append(args, "--ignore-cgroups")
	}
	return append(args, command...)
}

// Version returns the installed runsc's version output.
func (r *Runsc) Version(ctx context.Context) (string, error) {
	deadline := runscDeadline
	output, err := r.tools.Run(ctx, system.Runsc, []string{"--version"}, &deadline)
	return output.Stdout, err
}

// Status reports the container's state; found is false when runsc does not know it.
func (r *Runsc) Status(ctx context.Context, id ID) (status Status, found bool, err error) {
	output, err := r.run(ctx, []string{"list", "--format=json"})
	if err != nil {
		return "", false, err
	}
	status, found, err = StatusIn([]byte(output.Stdout), id)
	if err != nil {
		return "", false, &ListError{Source: err}
	}
	return status, found, nil
}

// Start starts a detached container from bundle, writing stdout and stderr to
// log. The caller keeps ownership of log and closes it after Start returns.
// Failure reports the last 8 KiB from logPath. Cancellation kills and reaps
// the runsc command; the boot owner must still close the partial sandbox.
func (r *Runsc) Start(ctx context.Context, id ID, bundle string, log *os.File, logPath string) error {
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	command := r.tools.Command(runCtx, system.Runsc)
	command.Args = append(command.Args, r.Args([]string{"run", "--detach", "--bundle", bundle, string(id)})...)
	command.Stdout = log
	command.Stderr = log
	if err := command.Start(); err != nil {
		return &system.SpawnError{Tool: system.Runsc, Source: err}
	}
	// The deadline starts after spawning, as it does for the Rust command.
	fired := make(chan struct{})
	timer := time.AfterFunc(runscDeadline, func() {
		defer close(fired)
		cancel()
	})
	err := command.Wait()
	if !timer.Stop() {
		<-fired
		return &system.DeadlineError{Tool: system.Runsc, Deadline: runscDeadline}
	}
	if ctx.Err() != nil {
		return &system.SpawnError{Tool: system.Runsc, Source: ctx.Err()}
	}
	if err == nil {
		return nil
	}
	var exit *exec.ExitError
	if !errors.As(err, &exit) {
		return &system.SpawnError{Tool: system.Runsc, Source: err}
	}
	written, err := os.ReadFile(logPath)
	if err != nil {
		return err
	}
	return &StartError{Message: system.Tail(contract.LossyUTF8(written), 8*1024)}
}

// Wait waits for a container to exit without a deadline. Cancellation kills
// and reaps the wait command, leaving the sandbox for its owner to close.
func (r *Runsc) Wait(ctx context.Context, id ID) error {
	output, err := r.tools.Output(ctx, system.Runsc, r.Args([]string{"wait", string(id)}), nil)
	if err != nil {
		return err
	}
	_, err = system.Accept(system.Runsc, output, []int{0})
	return err
}

// Pause suspends sandbox execution.
func (r *Runsc) Pause(ctx context.Context, id ID) error {
	_, err := r.run(ctx, []string{"pause", string(id)})
	return err
}

// Resume resumes suspended sandbox execution.
func (r *Runsc) Resume(ctx context.Context, id ID) error {
	_, err := r.run(ctx, []string{"resume", string(id)})
	return err
}

// Terminate asks every sandbox process to terminate and returns even a nonzero status.
func (r *Runsc) Terminate(ctx context.Context, id ID) (system.Output, error) {
	deadline := runscDeadline
	return r.tools.Output(ctx, system.Runsc, r.Args([]string{"kill", "--all", string(id), "TERM"}), &deadline)
}

// Delete deletes the container and kills whatever still runs.
func (r *Runsc) Delete(ctx context.Context, id ID) error {
	_, err := r.run(ctx, []string{"delete", "--force", string(id)})
	return err
}

const runscDeadline = 60 * time.Second

//go:embed runtime-release.json
var releaseManifest []byte

// run executes a bounded runsc command with the one shipped runtime profile.
func (r *Runsc) run(ctx context.Context, command []string) (system.Output, error) {
	deadline := runscDeadline
	return r.tools.Run(ctx, system.Runsc, r.Args(command), &deadline)
}
