//go:build linux

package sandbox

import (
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

type workingFixture string

func (w workingFixture) Directory() string { return string(w) }
func (w workingFixture) Image(volume machinewire.Volume) string {
	return filepath.Join(string(w), string(volume)+".ext4")
}

type leaseFixture struct{ releases int }

func (l *leaseFixture) Release() { l.releases++ }

type networkFixture struct {
	attachError error
	detachError error
	attached    bool
}

func (n *networkFixture) Attach(context.Context, Slot) error {
	n.attached = true
	return n.attachError
}
func (n *networkFixture) Detach(context.Context, Slot) error {
	if n.detachError != nil {
		return n.detachError
	}
	n.attached = false
	return nil
}

type diskFixture struct {
	copy     func(context.Context, string, string) error
	grow     func(context.Context, string) error
	capacity func(context.Context, string) (uint64, error)
}

func (d diskFixture) CloneSparse(ctx context.Context, source, destination string) error {
	return d.copy(ctx, source, destination)
}
func (d diskFixture) GrowMounted(ctx context.Context, path string) error {
	if d.grow != nil {
		return d.grow(ctx, path)
	}
	return errors.New("unexpected grow")
}
func (d diskFixture) Capacity(ctx context.Context, path string) (uint64, error) {
	if d.capacity != nil {
		return d.capacity(ctx, path)
	}
	return 0, errors.New("unexpected capacity")
}

// bootFixture supplies real loop-mounted ext4 images and a scripted runtime.
// It keeps mounts and automatic loop detachment under the test's namespace owner.
func bootFixture(ctx context.Context, t *testing.T, network *networkFixture) (*Sandbox, workingFixture, string, runnerwire.ManagedBoot, *leaseFixture) {
	t.Helper()
	root := t.TempDir()
	runtime := filepath.Join(root, "runtime")
	base := filepath.Join(root, "base")
	working := workingFixture(filepath.Join(root, "working"))
	for _, path := range []string{runtime, base, working.Directory()} {
		if err := os.Mkdir(path, 0700); err != nil {
			t.Fatal(err)
		}
	}
	tools, err := systemtest.OnPath(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		if _, err := tools.Run(ctx, system.Mke2fs, []string{"-q", "-t", "ext4", "-F", working.Image(volume), "32m"}, nil); err != nil {
			t.Fatal(err)
		}
	}
	boot, err := runnerwire.DecodeManagedBoot([]byte(`{"backendUrl":"https://backend.example.com","deviceToken":"private-token"}`))
	if err != nil {
		t.Fatal(err)
	}
	lease := &leaseFixture{}
	sandbox, err := New(Config{Runtime: runtime, BackendURL: boot.BackendURL}, Dependencies{Tools: tools, Runsc: fixtureRuntime(t, runtime), Network: network, Disks: diskFixture{copy: fixtureCopy}}, Slot{Index: 3, Namespace: "demi-3"}, lease)
	if err != nil {
		t.Fatal(err)
	}
	return sandbox, working, base, boot, lease
}

// cleanupBoot is deferred on the namespace thread, including after t.Fatal.
func cleanupBoot(ctx context.Context, t *testing.T, sandbox *Sandbox, working workingFixture, network *networkFixture) {
	t.Helper()
	network.detachError = nil
	if err := sandbox.Close(context.WithoutCancel(ctx), working); err != nil {
		t.Error(err)
	}
	if err := sandbox.StopWaiting(context.WithoutCancel(ctx)); err != nil {
		t.Error(err)
	}
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		if err := systemtest.WaitLoopDetach(context.WithoutCancel(ctx), working.Image(volume)); err != nil {
			t.Error(err)
		}
	}
}

func TestFailedStartRetainsRecordUntilCleanupSucceeds(t *testing.T) {
	isolatedSandbox(t, func(ctx context.Context) {
		refused := errors.New("network refused")
		network := &networkFixture{attachError: refused, detachError: refused}
		sandbox, working, base, boot, lease := bootFixture(ctx, t, network)
		defer cleanupBoot(ctx, t, sandbox, working, network)
		if err := sandbox.Start(ctx, working, base, boot); !errors.Is(err, refused) {
			t.Fatalf("start = %v", err)
		}
		data, err := os.ReadFile(filepath.Join(working.Directory(), "sandbox.json"))
		if err != nil {
			t.Fatal(err)
		}
		record, err := DecodeRecord(data)
		if err != nil || record.ID != sandbox.ID() || record.Slot != 3 {
			t.Fatalf("record = %+v, %v", record, err)
		}
		if strings.Contains(string(data), "private-token") {
			t.Fatal("credential persisted in record")
		}
		if err := sandbox.Close(ctx, working); !errors.Is(err, refused) {
			t.Fatalf("first close = %v", err)
		}
		if lease.releases != 0 {
			t.Fatal("failed close released slot")
		}
		if _, err := os.Stat(filepath.Join(working.Directory(), "sandbox.json")); err != nil {
			t.Fatal("failed close removed record", err)
		}
		network.detachError = nil
		if err := sandbox.Close(ctx, working); err != nil {
			t.Fatal(err)
		}
		if lease.releases != 1 || network.attached {
			t.Fatalf("cleanup: releases=%d attached=%v", lease.releases, network.attached)
		}
		if _, err := os.Stat(sandbox.directory.Root()); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("runtime remains: %v", err)
		}
		if _, err := os.Stat(filepath.Join(working.Directory(), "sandbox.json")); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("record remains: %v", err)
		}
	})
}

func TestCheckpointThawsAfterCopyFailureAndPanic(t *testing.T) {
	isolatedSandbox(t, func(ctx context.Context) {
		network := &networkFixture{}
		sandbox, working, base, boot, _ := bootFixture(ctx, t, network)
		defer cleanupBoot(ctx, t, sandbox, working, network)
		if err := sandbox.Start(ctx, working, base, boot); err != nil {
			t.Fatal(err)
		}
		if err := sandbox.Pause(ctx); err != nil {
			t.Fatal(err)
		}
		copies := workingFixture(t.TempDir())
		if result := sandbox.Capture(ctx, working, copies); result.Copied != nil || len(result.ThawErrors) != 0 {
			t.Fatalf("capture = %+v", result)
		}
		for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
			info, err := os.Stat(copies.Image(volume))
			if err != nil || info.Size() != 32<<20 {
				t.Fatalf("checkpoint copy %s: %v", volume, err)
			}
		}
		failure := errors.New("copy refused")
		for _, panics := range []bool{false, true} {
			sandbox.dependencies.Disks = diskFixture{copy: func(context.Context, string, string) error {
				if panics {
					panic(failure)
				}
				return failure
			}}
			result := sandbox.Capture(ctx, working, copies)
			if !errors.Is(result.Copied, failure) || len(result.ThawErrors) != 0 {
				t.Fatalf("capture failure = %+v", result)
			}
			for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
				thawed, err := system.Thaw(ctx, sandbox.directory.Volume(volume))
				if err != nil || thawed != system.NotFrozen {
					t.Fatalf("%s stayed frozen: %v, %v", volume, thawed, err)
				}
			}
		}
		if err := sandbox.Resume(ctx); err != nil {
			t.Fatal(err)
		}
		if status, found, err := sandbox.Status(ctx); err != nil || !found || status != Running {
			t.Fatalf("status = %v, %v, %v", status, found, err)
		}
		if err := sandbox.Pause(ctx); err != nil {
			t.Fatal(err)
		}
		if err := sandbox.Close(ctx, working); err != nil {
			t.Fatal(err)
		}
		trace, err := os.ReadFile(filepath.Join(sandbox.dependencies.Runsc.Root(), "trace"))
		if err != nil {
			t.Fatal(err)
		}
		// The independently owned wait process may append its event between commands.
		commands := strings.ReplaceAll(string(trace), "wait\n", "")
		if !strings.Contains(commands, "resume\nkill\n") || !strings.Contains(commands, "delete\n") {
			t.Fatalf("paused stop sequence:\n%s", trace)
		}
	})
}

func TestGrowRefreshesLiveLoopAndNeverShrinks(t *testing.T) {
	isolatedSandbox(t, func(ctx context.Context) {
		network := &networkFixture{}
		sandbox, working, base, boot, _ := bootFixture(ctx, t, network)
		defer cleanupBoot(ctx, t, sandbox, working, network)
		if _, err := sandbox.Grow(ctx, working, machinewire.VolumeHome, 64<<20); !errors.Is(err, ErrNotGrowable) {
			t.Fatalf("unstarted grow = %v", err)
		}
		if err := sandbox.Start(ctx, working, base, boot); err != nil {
			t.Fatal(err)
		}
		grows := 0
		sandbox.dependencies.Disks = diskFixture{
			grow: func(_ context.Context, device string) (err error) {
				grows++
				file, err := os.Open(device)
				if err != nil {
					return err
				}
				defer func() { err = errors.Join(err, file.Close()) }()
				size, err := file.Seek(0, io.SeekEnd)
				if err != nil {
					return err
				}
				if size != 64<<20 {
					t.Errorf("resize sees loop capacity %d", size)
				}
				return nil
			},
			// Deliberately distinguish filesystem capacity from requested file length.
			capacity: func(_ context.Context, image string) (uint64, error) {
				if image != working.Image(machinewire.VolumeHome) {
					t.Errorf("capacity image = %s", image)
				}
				return (64 << 20) - 4096, nil
			},
		}
		for _, requested := range []uint64{64 << 20, 32 << 20} {
			capacity, err := sandbox.Grow(ctx, working, machinewire.VolumeHome, requested)
			if err != nil || capacity != (64<<20)-4096 {
				t.Fatalf("grow = %d, %v", capacity, err)
			}
		}
		if grows != 2 {
			t.Fatalf("resize calls = %d", grows)
		}
		info, err := os.Stat(working.Image(machinewire.VolumeHome))
		if err != nil || info.Size() != 64<<20 {
			t.Fatalf("image size = %v, %v", info, err)
		}
	})
}
