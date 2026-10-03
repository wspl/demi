//go:build acceptance

package backendtest

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/machines/sandbox"
	"github.com/wspl/demi/internal/machinewire"
)

// ChromeProcesses reads independent OS evidence of Chrome executables under
// this test's artifact installation. ps preserves spaces in executable paths.
func ChromeProcesses(ctx context.Context, root string) ([]int, error) {
	if runtime.GOOS == "linux" {
		entries, err := os.ReadDir("/proc")
		if err != nil {
			return nil, fmt.Errorf("chrome processes: %w", err)
		}
		var processes []int
		for _, entry := range entries {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			pid, err := strconv.Atoi(entry.Name())
			if err != nil {
				continue
			}
			executable, err := os.Readlink(filepath.Join("/proc", entry.Name(), "exe"))
			// sysinfo likewise omits a process whose executable cannot be inspected,
			// including a process that exited during the snapshot.
			if err == nil && strings.HasPrefix(executable, root+"/") {
				processes = append(processes, pid)
			}
		}
		return processes, nil
	}

	output, err := exec.CommandContext(ctx, "ps", "-axo", "pid=,comm=").Output()
	if err != nil {
		return nil, fmt.Errorf("chrome processes: %w", err)
	}
	var processes []int
	for _, line := range strings.Split(string(output), "\n") {
		line = strings.TrimSpace(line)
		pid, executable, found := strings.Cut(line, " ")
		if !found || !strings.HasPrefix(strings.TrimSpace(executable), root+"/") {
			continue
		}
		number, err := strconv.Atoi(pid)
		if err != nil {
			return nil, fmt.Errorf("chrome pid: %w", err)
		}
		processes = append(processes, number)
	}
	return processes, nil
}

// RealImageState is the manager's generated committed-generation contract.
type RealImageState = machinewire.MachineImageState

// RealImage reads a committed generation without reconciling the manager when
// this observation connection closes, including after the backend has closed.
func RealImage(ctx context.Context, socket, device string) (*RealImageState, error) {
	life, cancel := context.WithCancel(ctx)
	client, _ := cloud.NewClient(life, socket)
	defer func() {
		cancel()
		// Cancellation deliberately closes this observer without reconciling Clouds.
		_ = client.Close(life)
	}()
	return cloud.Call(ctx, client, machinewire.ImageStateParams{DeviceID: device})
}

// RealCheckpoint asks the existing backend-owned manager connection to save a Cloud.
func RealCheckpoint(ctx context.Context, b *TestBackend, device string) error {
	_, err := cloud.Call(ctx, b.Backend.Services().Cloud.Machines, machinewire.CheckpointParams{DeviceID: device})
	return err
}

// RealBootID validates the manager's persisted sandbox record before inspecting
// or killing its Sentry; only the boot belonging to this scenario is selected.
func RealBootID(data, device string) (string, error) {
	bytes, err := os.ReadFile(filepath.Join(data, "working", device, "sandbox.json"))
	if err != nil {
		return "", err
	}
	record, err := sandbox.DecodeRecord(bytes)
	if err != nil {
		return "", err
	}
	return string(record.ID), nil
}
