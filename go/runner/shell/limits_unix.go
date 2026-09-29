//go:build unix

package shell

import (
	"bytes"
	"context"
	"fmt"
	"log/slog"
	"os/exec"
	"strconv"
	"strings"
	"sync"

	"github.com/wspl/demi/go/runner/process"
	"golang.org/x/sys/unix"
)

var inheritedFiles struct {
	once  sync.Once
	limit unix.Rlimit
	err   error
}

// readInheritedOpenFiles finds the limits children inherit, falling back to the
// runner's limits with a Host-log warning if the native probe fails.
func readInheritedOpenFiles(ctx context.Context, shellPath string) (unix.Rlimit, error) {
	limit, err := probeOpenFiles(ctx, shellPath)
	if err == nil {
		return limit, nil
	}
	slog.Warn("could not probe inherited open-file limits; using runner limits", "error", err)
	err = unix.Getrlimit(unix.RLIMIT_NOFILE, &limit)
	return limit, err
}

func probeOpenFiles(ctx context.Context, shellPath string) (unix.Rlimit, error) {
	// Go's syscall/rlimit.go (go.dev/issue/46279) raises NOFILE before our code
	// runs and keeps the original private, restoring it only in os/exec children.
	// A Go helper raises it again. This one native-shell probe observes the
	// inherited soft and hard limits; the ulimit builtin itself remains ours.
	var stdout, stderr bytes.Buffer
	cmd, err := process.Start(ctx, func() (*exec.Cmd, error) {
		// Start may be called only once per Cmd, even when it fails.
		cmd := exec.CommandContext(ctx, shellPath, "-c", "ulimit -Sn; ulimit -Hn")
		cmd.Env = []string{}
		cmd.Stdout = &stdout
		cmd.Stderr = &stderr
		return cmd, cmd.Start()
	})
	if err == nil {
		err = cmd.Wait()
	}
	if err != nil {
		return unix.Rlimit{}, fmt.Errorf("open-file limit probe: %w (stderr: %s)", err, strings.TrimSpace(stderr.String()))
	}
	fields := strings.Fields(stdout.String())
	if len(fields) != 2 {
		return unix.Rlimit{}, fmt.Errorf("open-file limit probe returned %q; want soft and hard limits", stdout.String())
	}
	var values [2]uint64
	for i, field := range fields {
		if field == "unlimited" {
			values[i] = infinity()
			continue
		}
		value, err := strconv.ParseUint(field, 10, 64)
		if err != nil {
			return unix.Rlimit{}, fmt.Errorf("open-file limit probe returned invalid limit %q: %w", field, err)
		}
		values[i] = value
	}
	if values[0] > values[1] {
		return unix.Rlimit{}, fmt.Errorf("open-file limit probe returned soft limit %d above hard limit %d", values[0], values[1])
	}
	return unix.Rlimit{Cur: values[0], Max: values[1]}, nil
}
