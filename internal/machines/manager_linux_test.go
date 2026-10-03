//go:build linux

package machines

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/machines/network"
	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/system/systemtest"
	"github.com/wspl/demi/internal/machinewire"
)

var rootTests = flag.Bool(
	"machines-root",
	false,
	"run Cloud storage and installer scenarios as root with filesystem tools",
)

// managerFixture exercises actual storage and workers independently of runtime processes.
func managerFixture(t *testing.T) *Manager {
	t.Helper()
	config, err := ParseConfig(
		nil,
		[]string{
			"DEMI_MANAGED_RUNSC=/nonexistent/runsc",
			"DEMI_MANAGED_IMAGE=/image",
			"DEMI_MANAGED_BACKEND_URL=http://203.0.113.10:3271",
			"DEMI_MANAGED_DNS=1.1.1.1",
			"DEMI_MACHINE_MANAGER_SOCKET=/socket",
			"DEMI_MACHINE_MANAGER_DATA=" + t.TempDir(),
			"DEMI_MANAGED_SYSTEM_MIB=32",
			"DEMI_MANAGED_HOME_MIB=32",
			"DEMI_MANAGED_SLOTS=8",
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	core := &Core{
		Config: config,
		Store:  storage.NewStore(config.Images()),
		Slots:  network.NewPool(config.Subnet, config.Slots),
	}
	base := machinewire.BaseVersion("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
	root := filepath.Join(core.Store.Bases(), string(base), "rootfs/etc/skel")
	for _, path := range []string{root, config.Working()} {
		if err = os.MkdirAll(path, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	for path, data := range map[string]string{
		filepath.Join(root, ".profile"):                                  "export EDITOR=vi\n",
		filepath.Join(core.Store.Bases(), string(base), "manifest.json"): "{}",
	} {
		if err = os.WriteFile(path, []byte(data), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if *rootTests {
		core.Tools, err = systemtest.OnPath(t.Context())
		if err != nil {
			t.Fatal(err)
		}
	}
	m := NewManager(core, base, make(chan machinewire.DeviceID, 4))
	t.Cleanup(func() {
		if err := m.Close(context.Background()); err != nil && !errors.Is(err, ErrClosed) {
			t.Error(err)
		}
	})
	return m
}

func stateOf(t *testing.T, m *Manager) *machinewire.MachineImageState {
	t.Helper()
	state, err := m.core.Store.Read(t.Context(), "dev-1")
	if err != nil {
		t.Fatal(err)
	}
	return state
}

func resetDevice(t *testing.T, m *Manager, operation string) {
	t.Helper()
	if _, err := m.Handle(
		t.Context(),
		&machinewire.Reset{
			Params: machinewire.ResetParams{DeviceID: "dev-1", OperationID: operation, BaseVersion: string(m.base)},
		},
	); err != nil {
		t.Fatal(err)
	}
}

// Cost: three 32 MiB sparse ext4 creations, normally <1 s in the VM.
func TestResetPublishesFreshSystemWithSavedHomeOnce(t *testing.T) {
	if !*rootTests {
		t.Skip("requires -machines-root and filesystem tools")
	}
	m := managerFixture(t)
	if stateOf(t, m) != nil {
		t.Fatal("unexpected first generation")
	}
	resetDevice(t, m, "op-1")
	first := stateOf(t, m)
	if first == nil || first.ResetID == nil || *first.ResetID != "op-1" || first.BaseVersion != m.base ||
		first.SystemBytes != 32<<20 ||
		first.HomeBytes != 32<<20 {
		t.Fatalf("state: %+v", first)
	}
	generations := filepath.Join(m.core.Config.Images(), "dev-1/generations")
	entries, err := os.ReadDir(generations)
	if err != nil || len(entries) != 2 {
		t.Fatalf("initial and reset generations: %v %v", entries, err)
	}
	images := m.core.Store.Images("dev-1", first.Generation)
	oldHome, err := os.Stat(images.Home)
	if err != nil {
		t.Fatal(err)
	}
	oldSystem, err := os.Stat(images.System)
	if err != nil {
		t.Fatal(err)
	}
	resetDevice(t, m, "op-1")
	if !reflect.DeepEqual(stateOf(t, m), first) {
		t.Fatal("reset was not idempotent")
	}
	resetDevice(t, m, "op-2")
	second := stateOf(t, m)
	if second.Generation == first.Generation || *second.ResetID != "op-2" {
		t.Fatal(second)
	}
	images = m.core.Store.Images("dev-1", second.Generation)
	home, err := os.Stat(images.Home)
	if err != nil {
		t.Fatal(err)
	}
	system, err := os.Stat(images.System)
	if err != nil {
		t.Fatal(err)
	}
	if !os.SameFile(oldHome, home) || os.SameFile(oldSystem, system) {
		t.Fatal("reset must replace system and hard-link retained home")
	}
	entries, err = os.ReadDir(generations)
	if err != nil || len(entries) != 2 {
		t.Fatalf("retained generations: %v %v", entries, err)
	}
	for _, entry := range entries {
		if entry.Name() != string(first.Generation) && entry.Name() != string(second.Generation) {
			t.Fatal(entry.Name())
		}
	}
	_, err = m.Handle(
		t.Context(),
		&machinewire.Reset{
			Params: machinewire.ResetParams{DeviceID: "dev-1", OperationID: "op-3", BaseVersion: "missing"},
		},
	)
	if err == nil || err.Error() != "Cloud base missing is not imported" {
		t.Fatal(err)
	}
	_, err = m.Handle(
		t.Context(),
		&machinewire.Reset{
			Params: machinewire.ResetParams{DeviceID: "dev-1", OperationID: "op-3", BaseVersion: "../b"},
		},
	)
	if err == nil {
		t.Fatal("invalid base accepted")
	}
	t.Run("invalid base message", func(t *testing.T) {
		t.Skip("fidelity 10: manager name errors omit the kind and offending value")
		if err.Error() != `invalid base version: "../b"` {
			t.Fatalf("invalid base: %v", err)
		}
	})
}

// Cost: sparse 32 MiB images and filesystem recovery, normally <1 s in the VM.
func TestRecoveryPublishesWorkingPairAndRemovesStages(t *testing.T) {
	if !*rootTests {
		t.Skip("requires -machines-root and filesystem tools")
	}
	m := managerFixture(t)
	resetDevice(t, m, "op-1")
	committed := stateOf(t, m)
	working := storage.NewWorkingPair(m.core.Config.Working(), "dev-1")
	if err := os.Mkdir(working.Directory(), 0o700); err != nil {
		t.Fatal(err)
	}
	sources := m.core.Store.Images("dev-1", committed.Generation)
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		if err := storage.CloneSparse(
			t.Context(),
			sources.ForVolume(volume),
			working.Images().ForVolume(volume),
		); err != nil {
			t.Fatal(err)
		}
	}
	if err := working.WriteManifest(t.Context(), *committed); err != nil {
		t.Fatal(err)
	}
	stage := filepath.Join(m.core.Config.Working(), ".wake-0f6c3d4e")
	if err := os.Mkdir(stage, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := fenceAndSave(t.Context(), m.core); err != nil {
		t.Fatal(err)
	}
	saved := stateOf(t, m)
	if saved.Generation == committed.Generation || !reflect.DeepEqual(saved.ResetID, committed.ResetID) {
		t.Fatal(saved)
	}
	for _, path := range []string{working.Directory(), stage} {
		if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("retained %s: %v", path, err)
		}
	}
	invalid := filepath.Join(m.core.Config.Working(), "not a device")
	if err := os.Mkdir(invalid, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(stage, 0o700); err != nil {
		t.Fatal(err)
	}
	nameErr := fenceAndSave(t.Context(), m.core)
	if nameErr == nil {
		t.Fatal("invalid device silently skipped")
	}
	t.Run("invalid recovery name message", func(t *testing.T) {
		t.Skip("fidelity 10: recovery name error omits the kind and offending value")
		if nameErr.Error() != `a working pair is not named by a device id: invalid device id: "not a device"` {
			t.Fatalf("recovery name: %v", nameErr)
		}
	})
	if _, err := os.Stat(stage); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("stage must be removed before any invalid device stops recovery: %v", err)
	}
	if err := os.Remove(invalid); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(working.Directory(), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(working.SandboxRecord(), []byte(`{"id":"demi-0f6c3d4e","slot":8}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := fenceAndSave(
		t.Context(),
		m.core,
	); err == nil ||
		err.Error() != "Existing Cloud slot exceeds configured pool" {
		t.Fatal(err)
	}
}

func TestStoppedDeviceOperationsAndShutdown(t *testing.T) {
	m := managerFixture(t)
	if _, err := m.Handle(
		t.Context(),
		&machinewire.Hibernate{Params: machinewire.HibernateParams{DeviceID: "dev-1"}},
	); err != nil {
		t.Fatal(err)
	}
	state, err := m.Handle(
		t.Context(),
		&machinewire.RuntimeStateCall{Params: machinewire.RuntimeStateParams{DeviceID: "dev-1"}},
	)
	if err != nil || string(state) != `"stopped"` {
		t.Fatalf("%s %v", state, err)
	}
	_, err = m.Handle(
		t.Context(),
		&machinewire.GrowVolume{
			Params: machinewire.GrowVolumeParams{DeviceID: "dev-1", Volume: machinewire.VolumeHome, Bytes: 1 << 30},
		},
	)
	if err == nil || err.Error() != "Cloud is not running" {
		t.Fatal(err)
	}
	if _, err = m.Handle(
		t.Context(),
		&machinewire.Hibernate{Params: machinewire.HibernateParams{DeviceID: "dev/1"}},
	); err == nil {
		t.Fatal("invalid device accepted")
	}
	t.Run("invalid device message", func(t *testing.T) {
		t.Skip("fidelity 10: manager name errors omit the kind and offending value")
		if err.Error() != `invalid device id: "dev/1"` {
			t.Fatalf("invalid device: %v", err)
		}
	})

	t.Run("reconcile", func(t *testing.T) {
		if !*rootTests {
			t.Skip("requires -machines-root and isolated Linux networking")
		}
		err := systemtest.Isolate(t.Context(), func(ctx context.Context) error {
			core, err := NewCore(m.core.Config, m.core.Tools)
			if err != nil {
				return err
			}
			m.core = core
			_, err = m.Handle(ctx, &machinewire.Reconcile{Params: machinewire.ReconcileParams{}})
			return err
		})
		if err != nil {
			t.Fatal(err)
		}
	})
	if err = m.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if _, err = m.Handle(
		t.Context(),
		&machinewire.Hibernate{Params: machinewire.HibernateParams{DeviceID: "dev-1"}},
	); !errors.Is(
		err,
		ErrClosed,
	) {
		t.Fatal(err)
	}
}

// A stopped server cannot consume deaths; shutdown must unblock an admitted
// operation reporting one before it waits for that operation's permit.
func TestShutdownUnblocksDeathBeforeWaitingForAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		m := NewManager(nil, "base", make(chan machinewire.DeviceID))
		release, err := m.admission.Enter(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		done := make(chan struct{})
		go func() {
			defer close(done)
			defer release()
			worker := &deviceWorker{manager: m, id: "device"}
			worker.reportDeath(t.Context())
		}()
		synctest.Wait()
		if err := m.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		<-done
	})
}

// Cost: one local socket exchange and an idle device worker, under one second.
func TestManagerUnitReplyIsNull(t *testing.T) {
	m := managerFixture(t)
	ctx, cancel := context.WithCancel(t.Context())
	path := filepath.Join(t.TempDir(), "sock")
	socket, err := BindSocket(ctx, path)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	done := make(chan struct{})
	go func() {
		defer close(done)
		Serve(ctx, socket, m, nil).Wait(context.Background())
	}()
	defer func() {
		cancel()
		<-done
	}()
	conn, err := net.Dial("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	// The test reads every reply it needs before the deferred close.
	defer func() { _ = conn.Close() }()
	if err := conn.SetDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}
	if _, err := conn.Write([]byte(`{"id":"7","op":"hibernate","params":{"deviceId":"dev-1"}}` + "\n")); err != nil {
		t.Fatal(err)
	}
	line, err := bufio.NewReader(conn).ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	if want := `{"type":"ok","id":"7","result":null}` + "\n"; line != want {
		t.Fatalf("reply: %s, want %s", line, want)
	}
}
