//go:build linux

package machines_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/go/machines/internal/network"
	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/runnerproto"
)

// The failure paths of a running device, with a runsc that is a shell script: it
// keeps one container's status in a directory, records each command it is run
// with, fails a command once when the test asks, and ends `wait` when the
// container is killed or the test says the container died. Everything else is
// real: the loop devices, the mounts, the network namespace, the freeze.

// stubRunsc is the script and the directory it keeps its state in.
type stubRunsc struct {
	t     *testing.T
	path  string
	state string
}

const stubScript = `#!/bin/sh
state=%q
command=
for argument in "$@"; do
	case "$argument" in
	--*) ;;
	*) command=$argument; break ;;
	esac
done
echo "$command" >> "$state/commands"
for last; do :; done
fail_once() {
	if [ -e "$state/fail-$1" ]; then
		rm "$state/fail-$1"
		echo "stub runsc: $1 failed" >&2
		exit 1
	fi
}
set_status() {
	echo "$1" > "$state/status.tmp"
	mv "$state/status.tmp" "$state/status"
}
case "$command" in
list)
	if [ -e "$state/status" ]; then
		printf '[{"id":"%%s","status":"%%s"}]' "$(cat "$state/id")" "$(cat "$state/status")"
	else
		echo null
	fi ;;
run)
	fail_once run
	echo "$last" > "$state/id"
	set_status running ;;
wait)
	read -r _ < "$state/exit"
	exit 0 ;;
pause) fail_once pause; set_status paused ;;
resume) fail_once resume; set_status running ;;
kill)
	set_status stopped
	: > "$state/exit" ;;
delete) rm -f "$state/status" ;;
esac
`

func newStubRunsc(t *testing.T) *stubRunsc {
	t.Helper()
	state := t.TempDir()
	if err := syscall.Mkfifo(filepath.Join(state, "exit"), 0o600); err != nil {
		t.Fatal(err)
	}
	stub := &stubRunsc{t: t, path: filepath.Join(t.TempDir(), "runsc"), state: state}
	if err := os.WriteFile(stub.path, []byte(fmt.Sprintf(stubScript, state)), 0o755); err != nil {
		t.Fatal(err)
	}
	return stub
}

// failOnce makes the next run of command fail.
func (s *stubRunsc) failOnce(command string) {
	s.t.Helper()
	if err := os.WriteFile(filepath.Join(s.state, "fail-"+command), nil, 0o600); err != nil {
		s.t.Fatal(err)
	}
}

// die makes the container exit by itself: its status says stopped, and its
// `wait` ends. The open of the pipe waits for that `wait` to be there.
func (s *stubRunsc) die() {
	s.t.Helper()
	if err := os.WriteFile(filepath.Join(s.state, "status"), []byte("stopped\n"), 0o600); err != nil {
		s.t.Fatal(err)
	}
	exit, err := os.OpenFile(filepath.Join(s.state, "exit"), os.O_WRONLY, 0)
	if err != nil {
		s.t.Fatal(err)
	}
	exit.Close()
}

// commands returns the runsc commands run so far, in order.
func (s *stubRunsc) commands() []string {
	s.t.Helper()
	data, err := os.ReadFile(filepath.Join(s.state, "commands"))
	if err != nil {
		s.t.Fatal(err)
	}
	return strings.Fields(string(data))
}

func (s *stubRunsc) ran(command string) bool {
	s.t.Helper()
	for _, ran := range s.commands() {
		if ran == command {
			return true
		}
	}
	return false
}

// runningFixture is a manager whose runsc is the stub, without the resource
// limits, on a network that has the manager's policy.
func runningFixture(t *testing.T, slots uint16) (*fixture, *stubRunsc) {
	t.Helper()
	stub := newStubRunsc(t)
	f := newFixtureWith(t, stub.path, slots, nil)
	t.Cleanup(func() {
		if err := f.manager.Close(context.Background()); err != nil {
			t.Errorf("closing the manager: %v", err)
		}
	})
	if _, err := f.call(machinesproto.ReconcileParams{}); err != nil {
		t.Fatal(err)
	}
	return f, stub
}

func (f *fixture) wake(device string) error {
	f.t.Helper()
	boot, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"http://203.0.113.10:3271","deviceToken":"token"}`))
	if err != nil {
		f.t.Fatal(err)
	}
	_, err = f.call(machinesproto.WakeParams{DeviceID: device, Boot: boot})
	return err
}

func (f *fixture) runtimeState(device string) machinesproto.RuntimeState {
	f.t.Helper()
	state, err := f.call(machinesproto.RuntimeStateParams{DeviceID: device})
	if err != nil {
		f.t.Fatal(err)
	}
	return state.(machinesproto.RuntimeState)
}

// heldByASandbox lists what a boot leaves behind when it is not closed: mounts
// under the runtime directory and the slot's namespace and interface.
func heldByASandbox(t *testing.T) []string {
	t.Helper()
	var held []string
	mounts, err := os.ReadFile("/proc/self/mountinfo")
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(string(mounts), "\n") {
		if strings.Contains(line, "/run/demi-machines/demi-") {
			held = append(held, line)
		}
	}
	entries, err := os.ReadDir("/run/demi-machines")
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "demi-") {
			held = append(held, "/run/demi-machines/"+entry.Name())
		}
	}
	links, err := os.ReadDir("/sys/class/net")
	if err != nil {
		t.Fatal(err)
	}
	for _, link := range links {
		if strings.HasPrefix(link.Name(), "veth") || strings.HasPrefix(link.Name(), "demi") {
			held = append(held, "link "+link.Name())
		}
	}
	return held
}

func TestAWakeThatFailsToStartReleasesEverythingItTookAndItsSlot(t *testing.T) {
	// Two slots: a lease that stays after a failure runs the pool out.
	f, stub := runningFixture(t, 2)
	for attempt := range 3 {
		stub.failOnce("run")
		err := f.wake("dev-1")
		var started interface{ Error() string }
		if !errors.As(err, &started) || err == nil || !strings.HasPrefix(err.Error(), "Cloud start failed") {
			t.Fatalf("attempt %d: %v", attempt, err)
		}
		if state := f.runtimeState("dev-1"); state != machinesproto.Stopped {
			t.Errorf("attempt %d: a failed start leaves the device %s", attempt, state)
		}
		if held := heldByASandbox(t); len(held) > 0 {
			t.Fatalf("attempt %d: still held: %v", attempt, held)
		}
		if errors.Is(err, network.ErrExhausted) {
			t.Fatalf("attempt %d: %v", attempt, err)
		}
	}
	// The device wakes once the runtime starts, on the slot the failures gave back.
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	if state := f.runtimeState("dev-1"); state != machinesproto.Running {
		t.Errorf("the device is %s", state)
	}
}

func TestHibernateStopsTheSandboxReleasesItsSlotAndSavesTheWorkingPair(t *testing.T) {
	f, stub := runningFixture(t, 1)
	for round := range 2 {
		if err := f.wake("dev-1"); err != nil {
			t.Fatalf("round %d: %v", round, err)
		}
		if _, err := f.call(machinesproto.HibernateParams{DeviceID: "dev-1"}); err != nil {
			t.Fatalf("round %d: %v", round, err)
		}
		if held := heldByASandbox(t); len(held) > 0 {
			t.Fatalf("round %d: still held: %v", round, held)
		}
	}
	if !stub.ran("kill") || !stub.ran("delete") {
		t.Errorf("the runtime was not stopped: %v", stub.commands())
	}
	if f.state("dev-1") == nil {
		t.Error("the working pair was not saved")
	}
}

func TestASandboxThatExitsByItselfIsReportedAndItsDeviceIsSavedAndStopped(t *testing.T) {
	f, stub := runningFixture(t, 1)
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	stub.die()
	if dead := <-f.deaths; dead != "dev-1" {
		t.Fatalf("the death of %q", dead)
	}
	if state := f.runtimeState("dev-1"); state != machinesproto.Stopped {
		t.Errorf("the device is %s", state)
	}
	if held := heldByASandbox(t); len(held) > 0 {
		t.Errorf("still held: %v", held)
	}
	if f.state("dev-1") == nil {
		t.Error("the working pair was not saved")
	}
	// Its slot is free again.
	if err := f.wake("dev-1"); err != nil {
		t.Errorf("waking again: %v", err)
	}
}

func TestACheckpointPublishesACopyAndKeepsTheSandboxRunning(t *testing.T) {
	f, stub := runningFixture(t, 1)
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	before := f.state("dev-1")
	if _, err := f.call(machinesproto.CheckpointParams{DeviceID: "dev-1"}); err != nil {
		t.Fatal(err)
	}
	after := f.state("dev-1")
	if after == nil || after.Generation == before.Generation {
		t.Errorf("no new generation: %+v", after)
	}
	if state := f.runtimeState("dev-1"); state != machinesproto.Running {
		t.Errorf("the device is %s", state)
	}
	// Paused for the copy, and resumed after it.
	commands := stub.commands()
	pause, resume := -1, -1
	for i, command := range commands {
		switch command {
		case "pause":
			pause = i
		case "resume":
			resume = i
		}
	}
	if pause < 0 || resume < pause {
		t.Errorf("the checkpoint ran %v", commands)
	}
}

func TestACheckpointThatCannotResumeStopsTheSandboxAndReportsItsDeath(t *testing.T) {
	f, stub := runningFixture(t, 1)
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	before := f.state("dev-1")
	stub.failOnce("resume")
	if _, err := f.call(machinesproto.CheckpointParams{DeviceID: "dev-1"}); err == nil {
		t.Fatal("the checkpoint succeeded")
	}
	if dead := <-f.deaths; dead != "dev-1" {
		t.Fatalf("the death of %q", dead)
	}
	if state := f.runtimeState("dev-1"); state != machinesproto.Stopped {
		t.Errorf("the device is %s", state)
	}
	if held := heldByASandbox(t); len(held) > 0 {
		t.Errorf("still held: %v", held)
	}
	if after := f.state("dev-1"); after == nil || !sameState(*after, *before) {
		t.Errorf("a failed checkpoint published %+v", after)
	}
}

func TestACheckpointThatCannotPauseFailsAndLeavesTheSandboxRunning(t *testing.T) {
	f, stub := runningFixture(t, 1)
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	stub.failOnce("pause")
	if _, err := f.call(machinesproto.CheckpointParams{DeviceID: "dev-1"}); err == nil {
		t.Fatal("the checkpoint succeeded")
	}
	if state := f.runtimeState("dev-1"); state != machinesproto.Running {
		t.Errorf("the device is %s", state)
	}
	if !stub.ran("resume") {
		t.Errorf("the sandbox was not resumed: %v", stub.commands())
	}
	select {
	case dead := <-f.deaths:
		t.Errorf("the death of %q", dead)
	default:
	}
}

func TestAGrowthNeverShrinksAVolumeOrTouchesItsRecord(t *testing.T) {
	f, _ := runningFixture(t, 1)
	if err := f.wake("dev-1"); err != nil {
		t.Fatal(err)
	}
	manifest := filepath.Join(f.data, "working", "dev-1", "manifest.json")
	before, err := os.ReadFile(manifest)
	if err != nil {
		t.Fatal(err)
	}
	identity := inode(t, manifest)
	for _, bytes := range []uint64{1, 16 << 20, 32 << 20} {
		if _, err := f.call(machinesproto.GrowVolumeParams{DeviceID: "dev-1", Volume: machinesproto.Home, Bytes: bytes}); err != nil {
			t.Fatalf("growing to %d: %v", bytes, err)
		}
	}
	after, err := os.ReadFile(manifest)
	if err != nil || !bytes.Equal(before, after) || inode(t, manifest) != identity {
		t.Errorf("the record of the working pair changed: %s, %v", after, err)
	}
}
