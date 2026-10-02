//go:build linux

package sandbox

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

// PrepareCgroups requires CPU, memory and PID controllers before changing the
// cgroup root, then enables them for sandbox cgroups. Do not call with limits off.
func PrepareCgroups(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	data, err := os.ReadFile(filepath.Join(cgroupRoot, "cgroup.controllers"))
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	available := strings.Fields(string(data))
	var missing []string
	for _, controller := range []string{"cpu", "memory", "pids"} {
		if !slices.Contains(available, controller) {
			missing = append(missing, controller)
		}
	}
	if len(missing) != 0 {
		return &MissingControllersError{Missing: missing}
	}
	enable := []byte("+cpu +memory +pids")
	if err := os.WriteFile(filepath.Join(cgroupRoot, "cgroup.subtree_control"), enable, 0666); err != nil {
		return err
	}
	root := filepath.Join(cgroupRoot, "demi-cloud")
	if err := os.MkdirAll(root, 0777); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(root, "cgroup.subtree_control"), enable, 0666)
}

// Fence kills all writers in id's cgroup, waits for it to empty and removes
// it, including a partial runtime runsc never recorded. Absence is accepted.
// Do not call with limits off.
func Fence(ctx context.Context, id ID) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	group := filepath.Join(cgroupRoot, "demi-cloud", string(id))
	if err := os.WriteFile(filepath.Join(group, "cgroup.kill"), []byte("1"), 0666); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		return err
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		data, err := os.ReadFile(filepath.Join(group, "cgroup.events"))
		if err != nil {
			return err
		}
		if slices.Contains(strings.Split(string(data), "\n"), "populated 0") {
			break
		}
		if !time.Now().Before(deadline) {
			return ErrWriters
		}
		timer := time.NewTimer(50 * time.Millisecond)
		select {
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		}
		timer.Stop()
	}
	return os.Remove(group)
}

const cgroupRoot = "/sys/fs/cgroup"
