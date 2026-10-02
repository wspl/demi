//go:build darwin || linux

package process

import (
	"context"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
	"sync"

	"golang.org/x/sys/unix"
)

var inheritedUmask = sync.OnceValue(readUmask)
var inheritedOpenFiles = sync.OnceValues(func() (unix.Rlimit, error) {
	return probeOpenFiles(context.Background())
})

// RaiseOpenFileLimit reports the launch soft descriptor limit and the current
// limit raised by Go at startup. It never sets the runner's limits. Call it
// during runner startup to cache the limits inherited by its children.
func RaiseOpenFileLimit() (before, after uint64, err error) {
	launch, err := inheritedOpenFiles()
	if err != nil {
		return 0, 0, err
	}
	var current unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &current); err != nil {
		return 0, 0, fmt.Errorf("read runner open-file limit: %w", err)
	}
	return launch.Cur, current.Cur, nil
}

// ChildLimit returns a resource's inherited soft and hard limits, using the
// runner's startup open-file limits for RLIMIT_NOFILE.
func ChildLimit(resource int) (soft, hard uint64, err error) {
	var limit unix.Rlimit
	if resource == unix.RLIMIT_NOFILE {
		limit, err = inheritedOpenFiles()
	} else {
		err = unix.Getrlimit(resource, &limit)
	}
	if err != nil {
		return 0, 0, fmt.Errorf("read child resource limit: %w", err)
	}
	return limit.Cur, limit.Max, nil
}

// probeOpenFiles asks a non-Go child for the launch limits restored by os/exec.
// Another Go executable would raise them again before it could report them.
func probeOpenFiles(ctx context.Context) (unix.Rlimit, error) {
	output, err := Start(ctx, func() ([]byte, error) {
		return exec.CommandContext(ctx, "/bin/sh", "-c", "ulimit -Sn && ulimit -Hn").Output()
	})
	if err != nil {
		return unix.Rlimit{}, fmt.Errorf("probe launch open-file limits: %w", err)
	}
	return parseOpenFiles(string(output))
}

// parseOpenFiles accepts precisely the shell's two newline-terminated limits.
func parseOpenFiles(output string) (unix.Rlimit, error) {
	fields := strings.Split(output, "\n")
	if len(fields) != 3 || fields[2] != "" {
		return unix.Rlimit{}, fmt.Errorf("invalid launch open-file limits")
	}
	var values [2]uint64
	for i, field := range fields[:2] {
		if field == "unlimited" {
			values[i] = uint64(unix.RLIM_INFINITY)
			continue
		}
		if field == "" {
			return unix.Rlimit{}, fmt.Errorf("empty launch open-file limit")
		}
		for _, digit := range field {
			if digit < '0' || digit > '9' {
				return unix.Rlimit{}, fmt.Errorf("invalid launch open-file limit")
			}
		}
		value, err := strconv.ParseUint(field, 10, 64)
		if err != nil {
			return unix.Rlimit{}, fmt.Errorf("parse launch open-file limit: %w", err)
		}
		values[i] = value
	}
	if values[0] > values[1] || values[1] > uint64(unix.RLIM_INFINITY) {
		return unix.Rlimit{}, fmt.Errorf("invalid launch open-file limit range")
	}
	return unix.Rlimit{Cur: values[0], Max: values[1]}, nil
}

// Umask reads and caches the runner's creation mask. Call it during startup
// before any jobs run; nothing in the runner changes the mask afterwards.
func Umask() uint32 { return inheritedUmask() }
