//go:build linux

package machines_test

import (
	"context"
	"encoding/json/v2"
	"errors"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/go/machines"
	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/roottest"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

// The manager's storage state machine without a sandbox: first-use storage, reset,
// recovery of what a crash left, and reconciliation. Root runs them in a throwaway
// namespace; the sandbox itself is acceptance's.

// fixture is a manager on a state directory with one imported base, whose skeleton
// holds a profile. runsc is never run.
type fixture struct {
	t       *testing.T
	data    string
	core    *machines.Core
	manager *machines.Manager
	base    machinesproto.BaseVersion
	deaths  chan machinesproto.DeviceID
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	roottest.Require(t)
	programs, err := tools.Resolve(os.Args[0])
	if err != nil {
		t.Skip(err)
	}
	directory := t.TempDir()
	data := filepath.Join(directory, "data")
	backend, err := url.Parse("http://203.0.113.10:3271")
	if err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Data:       data,
		Runsc:      "/nonexistent/runsc",
		Image:      filepath.Join(directory, "image"),
		BackendURL: backend,
		Limits:     &config.Limits{CPUs: 2, MemoryMiB: 2048},
		SystemMiB:  32,
		HomeMiB:    32,
		Subnet:     netip.MustParsePrefix("172.30.0.0/16"),
		Slots:      8,
		DNS:        []netip.Addr{netip.MustParseAddr("1.1.1.1")},
	}
	base := machinesproto.BaseVersion(strings.Repeat("b", 64))
	rootfs := filepath.Join(cfg.Images(), "bases", string(base), "rootfs")
	if err := os.MkdirAll(filepath.Join(rootfs, "etc/skel"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(rootfs, "etc/skel/.profile"), []byte("export EDITOR=vi\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(rootfs, "../manifest.json"), []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(cfg.Working(), 0o777); err != nil {
		t.Fatal(err)
	}
	core := machines.NewCore(cfg, programs)
	deaths := make(chan machinesproto.DeviceID, 4)
	return &fixture{
		t:       t,
		data:    data,
		core:    core,
		manager: machines.NewManager(context.Background(), core, base, deaths),
		base:    base,
		deaths:  deaths,
	}
}

func (f *fixture) call(call machinesproto.Call) (any, error) {
	return f.manager.Handle(context.Background(), call)
}

func (f *fixture) state(device string) *machinesproto.ImageState {
	f.t.Helper()
	result, err := f.call(machinesproto.ImageStateParams{DeviceID: device})
	if err != nil {
		f.t.Fatal(err)
	}
	// The reply carries the record as its JSON.
	data, err := json.Marshal(result)
	if err != nil {
		f.t.Fatal(err)
	}
	if string(data) == "null" {
		return nil
	}
	state, err := machinesproto.DecodeImageState(data)
	if err != nil {
		f.t.Fatal(err)
	}
	return &state
}

// sameState reports whether two records hold the same values.
func sameState(a, b machinesproto.ImageState) bool {
	first, _ := machinesproto.EncodeImageState(a)
	second, _ := machinesproto.EncodeImageState(b)
	return string(first) == string(second)
}

func (f *fixture) generations(device string) []string {
	f.t.Helper()
	entries, err := os.ReadDir(filepath.Join(f.data, "images", device, "generations"))
	if err != nil {
		f.t.Fatal(err)
	}
	var names []string
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	slices.Sort(names)
	return names
}

func reset(device, operation string, base machinesproto.BaseVersion) machinesproto.ResetParams {
	return machinesproto.ResetParams{DeviceID: device, OperationID: operation, BaseVersion: string(base)}
}

func inode(t *testing.T, path string) uint64 {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info.Sys().(*syscall.Stat_t).Ino
}

func TestAResetPublishesAFreshSystemWithTheSavedHomeOncePerOperation(t *testing.T) {
	f := newFixture(t)
	if state := f.state("dev-1"); state != nil {
		t.Fatalf("a device before its first storage: %+v", state)
	}
	if _, err := f.call(reset("dev-1", "op-1", f.base)); err != nil {
		t.Fatal(err)
	}
	first := f.state("dev-1")
	if first == nil || first.ResetID == nil || *first.ResetID != "op-1" || first.BaseVersion != f.base {
		t.Fatalf("the first reset: %+v", first)
	}
	if first.SystemBytes != 32<<20 || first.HomeBytes != 32<<20 {
		t.Errorf("capacities %d, %d", first.SystemBytes, first.HomeBytes)
	}
	// The first use made the initial pair, which the reset replaced but for home.
	if got := f.generations("dev-1"); len(got) != 2 {
		t.Errorf("generations %v", got)
	}
	images := f.core.Store.Images("dev-1", first.Generation)

	// The same operation again changes nothing.
	if _, err := f.call(reset("dev-1", "op-1", f.base)); err != nil {
		t.Fatal(err)
	}
	if again := f.state("dev-1"); again == nil || !sameState(*again, *first) {
		t.Errorf("the same operation again: %+v", again)
	}

	if _, err := f.call(reset("dev-1", "op-2", f.base)); err != nil {
		t.Fatal(err)
	}
	second := f.state("dev-1")
	if second == nil || second.ResetID == nil || *second.ResetID != "op-2" || second.Generation == first.Generation {
		t.Fatalf("the second reset: %+v", second)
	}
	resetImages := f.core.Store.Images("dev-1", second.Generation)
	if inode(t, resetImages.Home) != inode(t, images.Home) {
		t.Error("home is copied, not carried over")
	}
	if inode(t, resetImages.System) == inode(t, images.System) {
		t.Error("the system is not new")
	}
	// The current and the previous generation remain.
	want := []string{string(first.Generation), string(second.Generation)}
	slices.Sort(want)
	if got := f.generations("dev-1"); !slices.Equal(got, want) {
		t.Errorf("generations %v, want %v", got, want)
	}

	missing := machinesproto.BaseVersion(strings.Repeat("c", 64))
	_, err := f.call(reset("dev-1", "op-3", missing))
	if err == nil || err.Error() != "Cloud base "+string(missing)+" is not imported" {
		t.Errorf("a base that is not imported: %v", err)
	}
	invalid := reset("dev-1", "op-3", f.base)
	invalid.BaseVersion = "../b"
	var name *machinesproto.IDError
	if _, err := f.call(invalid); !errors.As(err, &name) {
		t.Errorf("an invalid base version: %v", err)
	}
}

func TestRecoveryPublishesTheWorkingPairACrashLeftAndRemovesStages(t *testing.T) {
	f := newFixture(t)
	if _, err := f.call(reset("dev-1", "op-1", f.base)); err != nil {
		t.Fatal(err)
	}
	committed := f.state("dev-1")
	// A crash after wake staged the working pair: its images, its record and a
	// stage beside it.
	working := filepath.Join(f.data, "working")
	pair := filepath.Join(working, "dev-1")
	if err := os.Mkdir(pair, 0o755); err != nil {
		t.Fatal(err)
	}
	images := f.core.Store.Images("dev-1", committed.Generation)
	for _, volume := range machinesproto.Volumes {
		data, err := os.ReadFile(images.Get(volume))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(pair, storage.ImageFile(volume)), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	record, err := machinesproto.EncodeImageState(*committed)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pair, "manifest.json"), record, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(working, ".wake-0f6c3d4e"), 0o755); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if err := machines.FenceAndSave(ctx, f.core); err != nil {
		t.Fatal(err)
	}
	saved := f.state("dev-1")
	if saved == nil || saved.Generation == committed.Generation || saved.ResetID == nil || *saved.ResetID != "op-1" {
		t.Errorf("the saved generation: %+v", saved)
	}
	for _, gone := range []string{pair, filepath.Join(working, ".wake-0f6c3d4e")} {
		if _, err := os.Stat(gone); !errors.Is(err, os.ErrNotExist) {
			t.Errorf("%s remains: %v", gone, err)
		}
	}

	// A working entry that names no device is an error, never skipped.
	if err := os.Mkdir(filepath.Join(working, "not a device"), 0o755); err != nil {
		t.Fatal(err)
	}
	var name *machinesproto.IDError
	if err := machines.FenceAndSave(ctx, f.core); !errors.As(err, &name) {
		t.Errorf("an entry that is no device: %v", err)
	}
	if err := os.Remove(filepath.Join(working, "not a device")); err != nil {
		t.Fatal(err)
	}

	// A runtime record outside the configured pool stops recovery.
	if err := os.Mkdir(pair, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pair, "sandbox.json"), []byte(`{"id":"demi-0f6c3d4e","slot":8}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := machines.FenceAndSave(ctx, f.core); !errors.Is(err, machines.ErrSlot) || err.Error() != "Existing Cloud slot exceeds configured pool" {
		t.Errorf("a slot outside the pool: %v", err)
	}
}

func TestOperationsOnAStoppedDeviceAnswerInOrder(t *testing.T) {
	f := newFixture(t)
	if _, err := f.call(machinesproto.HibernateParams{DeviceID: "dev-1"}); err != nil {
		t.Fatal(err)
	}
	if state, err := f.call(machinesproto.RuntimeStateParams{DeviceID: "dev-1"}); err != nil || state != machinesproto.Stopped {
		t.Fatalf("the runtime state: %v, %v", state, err)
	}
	_, err := f.call(machinesproto.GrowVolumeParams{DeviceID: "dev-1", Volume: machinesproto.Home, Bytes: 1 << 30})
	if err == nil || err.Error() != "Cloud is not running" {
		t.Errorf("growing a stopped device: %v", err)
	}
	var name *machinesproto.IDError
	if _, err := f.call(machinesproto.HibernateParams{DeviceID: "dev/1"}); !errors.As(err, &name) {
		t.Errorf("an invalid device id: %v", err)
	}
	// Reconciliation drains the devices, recovers and installs the policy.
	if _, err := f.call(machinesproto.ReconcileParams{}); err != nil {
		t.Fatal(err)
	}
	// After shutdown, requests are refused, and the death channel is closed.
	if err := f.manager.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	_, err = f.call(machinesproto.HibernateParams{DeviceID: "dev-1"})
	if err == nil || err.Error() != "the Cloud manager is stopping" {
		t.Errorf("a request after shutdown: %v", err)
	}
	if _, open := <-f.deaths; open {
		t.Error("the death channel is still open")
	}
}
