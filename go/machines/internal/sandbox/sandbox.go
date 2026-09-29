//go:build linux

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"time"

	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/network"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/runnerproto"
)

const (
	// stopGrace is how long sandbox processes get to exit when asked to
	// terminate.
	stopGrace = 3 * time.Second
	stopPoll  = 50 * time.Millisecond
)

// Refusals of a boot.
var (
	// ErrAllowlist means the boot record names a backend the manager may not
	// send a runner to.
	ErrAllowlist = errors.New("Cloud backend differs from configured allowlist")
	// ErrNotGrowable means growth needs the running sandbox's loop devices.
	ErrNotGrowable = errors.New("Cloud volume growth needs the running sandbox's loop devices")
)

// A NeedsRecoveryError means a working filesystem failed its check before a mount.
type NeedsRecoveryError struct {
	Volume  machinesproto.Volume
	Message string
}

func (e *NeedsRecoveryError) Error() string {
	return fmt.Sprintf("Cloud %s filesystem needs recovery: %s", e.Volume, e.Message)
}

// A SignalError means the sandbox's processes could not be asked to terminate.
type SignalError struct {
	Message string
}

func (e *SignalError) Error() string { return "Cannot signal Cloud sandbox: " + e.Message }

// A Host is what a sandbox uses of the manager, all of it fixed at startup.
type Host struct {
	Config  *config.Config
	Tools   *tools.Tools
	Runsc   *Runsc
	Network *network.Network
	Cgroups *Cgroups
	Slots   *network.SlotPool
}

// A waiter is the goroutine that runs runsc wait, which ends when the sandbox
// exits.
type waiter struct {
	cancel context.CancelFunc
	// exit delivers how the wait ended, once.
	exit chan error
	// finished is closed when the goroutine has ended.
	finished chan struct{}
}

// A Sandbox is one boot and what it holds.
type Sandbox struct {
	host      *Host
	record    Record
	slot      network.Slot
	lease     *network.SlotLease
	directory *RuntimeDirectory
	// loops are the loop device numbers of the mounted images, for growth.
	loops  *storage.ImagePair[uint32]
	waiter *waiter
}

// New returns a new boot on lease's slot.
func New(host *Host, lease *network.SlotLease) *Sandbox {
	record := Record{ID: NewSandboxID(), Slot: lease.Slot().Index}
	return &Sandbox{
		host:      host,
		record:    record,
		slot:      lease.Slot(),
		lease:     lease,
		directory: NewRuntimeDirectory(host.Config.Runtime(), string(record.ID)),
	}
}

// Recorded returns the boot record names, as recovery finds it after a crash.
func Recorded(host *Host, record Record) *Sandbox {
	return &Sandbox{
		host:      host,
		record:    record,
		slot:      host.Slots.Slot(record.Slot),
		directory: NewRuntimeDirectory(host.Config.Runtime(), string(record.ID)),
	}
}

// Start starts the boot on the working pair with the base at base. ctx must
// outlive the sandbox: the wait for its exit runs under it. On an error the
// caller closes the sandbox, which releases what it holds.
func (s *Sandbox) Start(ctx context.Context, working *storage.WorkingPair, base string, boot runnerproto.ManagedBoot) error {
	backend, err := boot.BackendURL.Normal()
	if err != nil || backend != s.host.Config.BackendURL.String() {
		return ErrAllowlist
	}
	record, err := EncodeRecord(s.record)
	if err != nil {
		return err
	}
	if err := storage.WriteRecord(working.SandboxRecord(), record); err != nil {
		return err
	}
	if err := storage.Sync(working.Directory()); err != nil {
		return err
	}
	fault.Point("sandbox-record")
	if err := s.directory.Create(); err != nil {
		return err
	}
	if err := linux.Bind(base, s.directory.Base()); err != nil {
		return err
	}
	if err := linux.RemountReadOnly(s.directory.Base()); err != nil {
		return err
	}
	fault.Point("base-bound")
	var loops [2]uint32
	for i, volume := range machinesproto.Volumes {
		image := working.Images().Get(volume)
		if err := s.check(ctx, volume, image); err != nil {
			return err
		}
		if loops[i], err = s.mountVolume(volume, image); err != nil {
			return err
		}
		fault.Point("volume-mounted")
	}
	s.loops = &storage.ImagePair[uint32]{System: loops[0], Home: loops[1]}
	if err := linux.MountSystemOverlay(s.directory.Base(), s.directory.Volume(machinesproto.System), s.directory.RootFS()); err != nil {
		return err
	}
	if err := linux.MountTmpfs(s.directory.Credentials(), "size=1m,mode=0700"); err != nil {
		return err
	}
	if err := s.directory.WriteCredentials(boot, s.host.Config.DNS); err != nil {
		return err
	}
	fault.Point("credentials-written")
	if err := s.host.Network.Attach(ctx, s.slot); err != nil {
		return err
	}
	fault.Point("network-attached")
	if err := s.writeBundle(); err != nil {
		return err
	}
	if err := s.runsc(ctx); err != nil {
		return err
	}
	fault.Point("sandbox-started")
	s.startWaiter(ctx)
	return nil
}

// check runs e2fsck -p before a mount; exit 1 means it corrected the filesystem.
func (s *Sandbox) check(ctx context.Context, volume machinesproto.Volume, image string) error {
	output, err := s.host.Tools.Output(ctx, tools.E2fsck, []string{"-p", image}, 0)
	if err != nil {
		return err
	}
	if output.Code != 0 && output.Code != 1 {
		return &NeedsRecoveryError{Volume: volume, Message: output.Message()}
	}
	return nil
}

// mountVolume attaches image to a loop device and mounts its filesystem under
// the runtime directory. Unmounting the filesystem, or this mount failing,
// detaches the device; the sandbox keeps the device's number for growth.
func (s *Sandbox) mountVolume(volume machinesproto.Volume, image string) (uint32, error) {
	device, err := linux.AttachLoop(image)
	if err != nil {
		return 0, err
	}
	defer device.Close()
	if err := linux.MountExt4(device.Path(), s.directory.Volume(volume)); err != nil {
		return 0, err
	}
	return device.Number(), nil
}

// writeBundle writes the boot's OCI configuration.
func (s *Sandbox) writeBundle() error {
	var cgroup *CgroupLimits
	if limits := s.host.Config.Limits; limits != nil {
		cgroup = &CgroupLimits{Name: string(s.record.ID), Limits: *limits}
	}
	spec := Spec(Boot{Directory: s.directory, Namespace: s.slot.Namespace(), Cgroup: cgroup})
	data, err := Config(spec)
	if err != nil {
		return err
	}
	return storage.WriteRecord(s.directory.Config(), data)
}

// runsc starts the sandbox, with the bundle's log for its output.
func (s *Sandbox) runsc(ctx context.Context) error {
	log, err := os.OpenFile(s.directory.Log(), os.O_WRONLY|os.O_APPEND|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	defer log.Close()
	return s.host.Runsc.Start(ctx, s.record.ID, s.directory.Root(), log)
}

// startWaiter runs runsc wait in a goroutine of its own, which ends when the
// sandbox exits, when [Sandbox.Abandon] cancels it, or when ctx is done.
func (s *Sandbox) startWaiter(ctx context.Context) {
	ctx, cancel := context.WithCancel(ctx)
	w := &waiter{cancel: cancel, exit: make(chan error, 1), finished: make(chan struct{})}
	s.waiter = w
	id := s.record.ID
	runsc := s.host.Runsc
	go func() {
		defer close(w.finished)
		defer cancel()
		w.exit <- runsc.Wait(ctx, id)
	}()
}

// Exited returns a channel that delivers how the sandbox's exit was observed:
// once, when the sandbox exits. It is nil for a sandbox without a waiter, which
// never exits. After a receive the caller calls [Sandbox.ExitObserved].
func (s *Sandbox) Exited() <-chan error {
	if s.waiter == nil {
		return nil
	}
	return s.waiter.exit
}

// ExitObserved records that the exit was received from [Sandbox.Exited]: the
// waiter has ended.
func (s *Sandbox) ExitObserved() {
	s.waiter = nil
}

// Abandon ends the wait for the sandbox's exit, which kills runsc wait and leaves
// the sandbox itself to the stop-post recovery. A sandbox is abandoned only when
// the manager exits after a failed drain.
func (s *Sandbox) Abandon() {
	if s.waiter != nil {
		s.waiter.cancel()
		s.waiter = nil
	}
}

// Pause pauses the sandbox's tasks.
func (s *Sandbox) Pause(ctx context.Context) error {
	return s.host.Runsc.Pause(ctx, s.record.ID)
}

// Resume resumes the sandbox's tasks.
func (s *Sandbox) Resume(ctx context.Context) error {
	return s.host.Runsc.Resume(ctx, s.record.ID)
}

// A Capture is what a checkpoint's frozen window produced: whether the copies
// were made, and each thaw that failed.
type Capture struct {
	Copied     error
	ThawErrors []error
}

// Capture is the frozen window of a checkpoint, as one job on a locked thread:
// freeze both filesystems, copy both working images into stage and sync the
// copies, then thaw. The guard thaws on every path, a panic included. Nothing
// inside the window waits for another task or process, and nothing in it logs to
// a file on a frozen filesystem.
func (s *Sandbox) Capture(working *storage.WorkingPair, stage string) Capture {
	var capture Capture
	images := working.Images()
	copies := storage.ImagesIn(stage)
	// The job never fails: the capture carries its errors.
	_ = linux.OnThread(func() error {
		var frozen linux.Frozen
		defer frozen.Release()
		capture.Copied = func() error {
			for _, volume := range machinesproto.Volumes {
				if err := frozen.Freeze(s.directory.Volume(volume)); err != nil {
					return err
				}
			}
			fault.Point("frozen")
			for _, volume := range machinesproto.Volumes {
				if err := storage.CloneSparse(images.Get(volume), copies.Get(volume)); err != nil {
					return err
				}
				if err := storage.Sync(copies.Get(volume)); err != nil {
					return err
				}
			}
			return nil
		}()
		capture.ThawErrors = frozen.ThawAll()
		return nil
	})
	return capture
}

// Grow grows volume's filesystem to at least bytes: it extends the image (never
// shrinks it), refreshes the loop device the sandbox holds and grows the mounted
// filesystem. It returns the filesystem's capacity.
func (s *Sandbox) Grow(ctx context.Context, working *storage.WorkingPair, volume machinesproto.Volume, bytes uint64) (uint64, error) {
	if s.loops == nil {
		return 0, ErrNotGrowable
	}
	number := s.loops.Get(volume)
	image := working.Images().Get(volume)
	if err := extendImage(image, bytes, number); err != nil {
		return 0, err
	}
	fault.Point("capacity-refreshed")
	if err := storage.GrowMounted(ctx, s.host.Tools, linux.LoopPath(number)); err != nil {
		return 0, err
	}
	if err := storage.Sync(image); err != nil {
		return 0, err
	}
	return storage.Capacity(image)
}

// extendImage extends the image file to at least bytes and refreshes the loop
// device that holds it.
func extendImage(image string, bytes uint64, loop uint32) error {
	file, err := os.OpenFile(image, os.O_WRONLY, 0)
	if err != nil {
		return err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return err
	}
	if uint64(info.Size()) < bytes {
		if err := file.Truncate(int64(bytes)); err != nil {
			return err
		}
	}
	fault.Point("image-extended")
	return linux.RefreshLoopCapacity(loop)
}

// Close stops the boot and releases everything it holds, in reverse order of
// acquisition; each step tolerates a resource that is already gone. A failure
// keeps the sandbox, so a later operation retries the close.
func (s *Sandbox) Close(ctx context.Context, working *storage.WorkingPair) error {
	// A Gofer may be waiting on a frozen filesystem; thaw before signals.
	for _, volume := range machinesproto.Volumes {
		if err := thawIfMounted(s.directory.Volume(volume)); err != nil {
			return err
		}
	}
	if err := s.stopRuntime(ctx); err != nil {
		return err
	}
	if w := s.waiter; w != nil {
		s.waiter = nil
		// The sandbox was stopped; how its wait ended does not matter.
		<-w.finished
	}
	// Without the limits the sandbox has no cgroup; deleting the runtime ended
	// its Sentry and Gofer.
	if s.host.Config.Limits != nil {
		if err := s.host.Cgroups.Fence(ctx, s.record.ID); err != nil {
			return err
		}
	}
	if err := s.unmountAll(); err != nil {
		return err
	}
	if err := s.host.Network.Detach(ctx, s.slot); err != nil {
		return err
	}
	if err := s.directory.Remove(); err != nil {
		return err
	}
	if err := os.Remove(working.SandboxRecord()); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	if err := storage.Sync(working.Directory()); err != nil {
		return err
	}
	if s.lease != nil {
		s.lease.Release()
	}
	return nil
}

// stopRuntime asks every sandbox process to terminate, waits for them briefly,
// and deletes the runtime.
func (s *Sandbox) stopRuntime(ctx context.Context) error {
	runsc := s.host.Runsc
	id := s.record.ID
	status, known, err := runsc.Status(ctx, id)
	if err != nil {
		return err
	}
	if known && status != Stopped {
		if status == Paused {
			if err := runsc.Resume(ctx, id); err != nil {
				return err
			}
		}
		signalled, err := runsc.Terminate(ctx, id)
		if err != nil {
			return err
		}
		if !signalled.Success() {
			after, known, err := runsc.Status(ctx, id)
			if err != nil {
				return err
			}
			if !known || after != Stopped {
				return &SignalError{Message: signalled.Message()}
			}
		}
		if err := s.waitForExit(ctx); err != nil {
			return err
		}
	}
	_, known, err = runsc.Status(ctx, id)
	if err != nil {
		return err
	}
	if known {
		return runsc.Delete(ctx, id)
	}
	return nil
}

// waitForExit polls the sandbox's status until it stops running or the grace
// period passes.
func (s *Sandbox) waitForExit(ctx context.Context) error {
	deadline := time.Now().Add(stopGrace)
	for time.Now().Before(deadline) {
		status, _, err := s.host.Runsc.Status(ctx, s.record.ID)
		if err != nil {
			return err
		}
		if status != Running {
			return nil
		}
		select {
		case <-time.After(stopPoll):
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return nil
}

// thawIfMounted thaws the filesystem mounted at mount, if one is.
func thawIfMounted(mount string) error {
	root, err := linux.IsMountRoot(mount)
	if err != nil || !root {
		return err
	}
	_, err = linux.Thaw(mount)
	return err
}

// unmountAll unmounts the boot's mounts, the overlay first; the working
// filesystems are thawed first, and unmounting them detaches their loop devices.
func (s *Sandbox) unmountAll() error {
	for _, name := range MountPoints {
		path := filepath.Join(s.directory.Root(), name)
		root, err := linux.IsMountRoot(path)
		if err != nil {
			return err
		}
		if !root {
			continue
		}
		if name == "system" || name == "home" {
			if _, err := linux.Thaw(path); err != nil {
				return err
			}
		}
		if err := linux.Unmount(path); err != nil {
			return err
		}
	}
	return nil
}
