package sandbox

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/go/machines/internal/tools"
)

// ProfileFlags are the runtime profile: systrap, gVisor's own network stack in
// the slot's namespace, no temporary root overlay (system writes land in the
// device's image), shared file access so the images can be saved while the
// sandbox is paused, and setuid programs for sudo.
var ProfileFlags = [...]string{
	"--platform=systrap",
	"--network=sandbox",
	"--overlay2=none",
	"--file-access=shared",
	"--file-access-mounts=shared",
	"--allow-suid=true",
	"--directfs=true",
}

// commandDeadline is how long a runsc command may take: one that does not finish
// in this time has hung.
const commandDeadline = 60 * time.Second

// releaseJSON is the pinned runtime release, a copy of
// crates/machines/runtime/release.json until the Rust manager is gone.
//
//go:embed release.json
var releaseJSON []byte

// A RuntimeRelease is the runtime release the manager is built with.
type RuntimeRelease struct {
	Upstream           string `json:"upstream"`
	Commit             string `json:"commit"`
	ARM64Version       string `json:"arm64Version"`
	ARM64PatchSHA256   string `json:"arm64PatchSha256"`
	AMD64ArchiveSHA512 string `json:"amd64ArchiveSha512"`
	Bazel              string `json:"bazel"`
	BazelARM64SHA256   string `json:"bazelArm64Sha256"`
}

// PinnedRelease returns the pinned runtime release.
func PinnedRelease() RuntimeRelease {
	var release RuntimeRelease
	if err := json.Unmarshal(releaseJSON, &release, json.RejectUnknownMembers(true)); err != nil {
		// The release is embedded at build time; a malformed one is a build
		// defect.
		panic("the pinned runtime release is invalid: " + err.Error())
	}
	return release
}

// Version returns the version runsc reports on this architecture: arm64 runs the
// build with the seccomp trap fix, amd64 the upstream release.
func (r RuntimeRelease) Version() string {
	if runtime.GOARCH == "arm64" {
		return r.ARM64Version
	}
	return "release-" + r.Upstream
}

// ReportsVersion reports whether runsc --version printed exactly version on its
// first line.
func ReportsVersion(output, version string) bool {
	first, _, _ := strings.Cut(output, "\n")
	return first == "runsc version "+version
}

// A Status is a container's state, as runsc list reports it.
type Status string

// The states of a container.
const (
	Creating Status = "creating"
	Created  Status = "created"
	Running  Status = "running"
	Paused   Status = "paused"
	Stopped  Status = "stopped"
)

// StatusIn returns the status of container id in runsc list --format=json
// output, which is null when there are no containers; ok is false when the
// listing does not name it.
func StatusIn(listing string, id SandboxID) (status Status, ok bool, err error) {
	var containers []struct {
		ID     string `json:"id"`
		Status Status `json:"status"`
	}
	if err := json.Unmarshal([]byte(listing), &containers); err != nil {
		return "", false, err
	}
	for _, container := range containers {
		switch container.Status {
		case Creating, Created, Running, Paused, Stopped:
		default:
			return "", false, fmt.Errorf("unknown container status %q", container.Status)
		}
	}
	for _, container := range containers {
		if container.ID == string(id) {
			return container.Status, true, nil
		}
	}
	return "", false, nil
}

// Runsc drives runsc, with its state under the manager's runtime directory.
type Runsc struct {
	tools *tools.Tools
	root  string
	// cgroups is whether runsc sets up each sandbox's cgroup: only with the
	// resource limits on (docs/cloud/managed-hosts.md § Resource limits).
	cgroups bool
}

// NewRunsc returns runsc with its state under runtime, setting up cgroups or not.
func NewRunsc(t *tools.Tools, runtime string, cgroups bool) *Runsc {
	return &Runsc{tools: t, root: filepath.Join(runtime, "runsc"), cgroups: cgroups}
}

// Root returns where runsc keeps its containers' state.
func (r *Runsc) Root() string { return r.root }

// A StartError means runsc failed to start the sandbox; it carries the end of the
// runtime log.
type StartError struct {
	Log string
}

func (e *StartError) Error() string { return "Cloud start failed: " + e.Log }

// ListError means runsc's listing could not be read.
type ListError struct {
	Err error
}

func (e *ListError) Error() string { return "Cannot inspect Cloud runtimes: " + e.Err.Error() }

func (e *ListError) Unwrap() error { return e.Err }

// Args returns the arguments of command: the state root, the profile, the
// command. Without cgroups the profile says so: runsc gives a configuration
// without a cgroup path one of its own.
func (r *Runsc) Args(command ...string) []string {
	args := []string{"--root=" + r.root}
	args = append(args, ProfileFlags[:]...)
	if !r.cgroups {
		args = append(args, "--ignore-cgroups")
	}
	return append(args, command...)
}

func (r *Runsc) run(ctx context.Context, command ...string) (tools.Output, error) {
	return r.tools.Run(ctx, tools.Runsc, r.Args(command...), commandDeadline)
}

// Version returns the installed runsc's version output.
func (r *Runsc) Version(ctx context.Context) (string, error) {
	output, err := r.tools.Run(ctx, tools.Runsc, []string{"--version"}, commandDeadline)
	return output.Stdout, err
}

// Status returns the status of container id; ok is false when runsc does not
// know it.
func (r *Runsc) Status(ctx context.Context, id SandboxID) (status Status, ok bool, err error) {
	output, err := r.run(ctx, "list", "--format=json")
	if err != nil {
		return "", false, err
	}
	status, ok, err = StatusIn(output.Stdout, id)
	if err != nil {
		return "", false, &ListError{Err: err}
	}
	return status, ok, nil
}

// Start starts container id from bundle, detached. runsc and the sandbox write to
// the bundle's log file: a detached sandbox keeps its standard output, so a pipe
// would never reach its end.
func (r *Runsc) Start(ctx context.Context, id SandboxID, bundle string, log *os.File) error {
	ctx, cancel := context.WithTimeout(ctx, commandDeadline)
	defer cancel()
	command := r.tools.Command(ctx, tools.Runsc, r.Args("run", "--detach", "--bundle", bundle, string(id))...)
	command.Stdout = log
	command.Stderr = log
	err := command.Run()
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return &tools.Error{Tool: tools.Runsc, Kind: tools.Deadline, Limit: commandDeadline}
	}
	if err == nil {
		return nil
	}
	if command.ProcessState == nil {
		return &tools.Error{Tool: tools.Runsc, Kind: tools.Spawn, Err: err}
	}
	written, readErr := os.ReadFile(log.Name())
	if readErr != nil {
		return readErr
	}
	return &StartError{Log: tools.Tail(string(bytes.ToValidUTF8(written, []byte("�"))), 8*1024)}
}

// Wait waits for container id to exit, as long as it runs.
func (r *Runsc) Wait(ctx context.Context, id SandboxID) error {
	output, err := r.tools.Output(ctx, tools.Runsc, r.Args("wait", string(id)), 0)
	if err != nil {
		return err
	}
	_, err = tools.Accept(tools.Runsc, output, 0)
	return err
}

// Pause pauses container id.
func (r *Runsc) Pause(ctx context.Context, id SandboxID) error {
	_, err := r.run(ctx, "pause", string(id))
	return err
}

// Resume resumes container id.
func (r *Runsc) Resume(ctx context.Context, id SandboxID) error {
	_, err := r.run(ctx, "resume", string(id))
	return err
}

// Terminate asks every process of container id to terminate.
func (r *Runsc) Terminate(ctx context.Context, id SandboxID) (tools.Output, error) {
	return r.tools.Output(ctx, tools.Runsc, r.Args("kill", "--all", string(id), "TERM"), commandDeadline)
}

// Delete deletes container id, killing what still runs.
func (r *Runsc) Delete(ctx context.Context, id SandboxID) error {
	_, err := r.run(ctx, "delete", "--force", string(id))
	return err
}
