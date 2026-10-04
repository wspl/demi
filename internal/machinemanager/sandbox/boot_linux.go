//go:build linux

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"math"
	"net/netip"
	"os"
	"path/filepath"
	"slices"
	"sync"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/runnerproto"
)

// Config supplies trusted manager configuration. Limits is nil only when the
// operator explicitly disables resource limits; there is no fallback mode.
type Config struct {
	// Runtime is the runtime bundle directory.
	Runtime string
	// BackendURL is the only allowed sandbox backend.
	BackendURL runnerproto.BackendURL
	// DNS lists the sandbox IPv4 resolvers.
	DNS []netip.Addr
	// Limits is nil when resource limits are disabled.
	Limits *Limits
}

// Slot names a network allocation made by the manager, without choosing policy.
type Slot struct {
	// Index identifies the reserved network slot.
	Index uint16
	// Namespace is the name under /run/netns.
	Namespace string
}

// Lease is the network slot reservation transferred to a new boot. Release
// must be idempotent and must not wait. Recovery does not acquire a lease.
type Lease interface {
	Release()
}

// Network attaches and detaches the manager-selected slot. Its implementation
// owns firewall policy and makes Detach tolerate already absent resources.
type Network interface {
	Attach(ctx context.Context, slot Slot) error
	Detach(ctx context.Context, slot Slot) error
}

// Disks supplies image operations owned by storage, without exposing its store.
// CloneSparse preserves holes; GrowMounted grows ext4 on the loop-device path;
// Capacity reads the image's actual nonzero filesystem capacity.
// Calls finish their disk operation before returning on cancellation.
type Disks interface {
	CloneSparse(ctx context.Context, source, destination string) error
	GrowMounted(ctx context.Context, device string) error
	Capacity(ctx context.Context, image string) (uint64, error)
}

// Dependencies supplies the infrastructure a boot uses. The manager retains
// ownership of these services for the lifetime of every sandbox using them.
type Dependencies struct {
	// Tools holds the resolved infrastructure executables.
	Tools *system.Tools
	// Runsc runs the pinned sandbox runtime.
	Runsc *Runsc
	// Network attaches the manager-selected network slot.
	Network Network
	// Disks supplies storage image operations.
	Disks Disks
}

// Images supplies paths for the two volumes. Storage owns image naming; the
// manager adapts its image pair to this boundary without copying that rule.
type Images interface {
	Image(volume machinemanagerproto.Volume) string
}

// Working supplies the private working images and their containing directory.
// Sandbox owns sandbox.json within Directory; storage owns image publication.
type Working interface {
	Images
	Directory() string
}

// Sandbox owns one boot and the resources it acquires. Its device worker
// serializes operations; Exited may wait concurrently with Close or StopWaiting.
// After a failed Start the owner must Close, even if Start was canceled.
type Sandbox struct {
	config       Config
	dependencies Dependencies
	record       Record
	slot         Slot
	lease        Lease
	directory    RuntimeDirectory
	loops        *[2]uint32
	// mu protects the waiter pointer only; no IO or join holds it.
	mu     sync.Mutex
	waiter *bootWaiter
}

type bootWaiter struct {
	cancel context.CancelFunc
	done   chan struct{}
	// err is immutable after done closes.
	err error
}

// New creates a boot on slot and takes ownership of lease on success. On an
// error the caller still owns lease. Config and slot are copied.
func New(config Config, dependencies Dependencies, slot Slot, lease Lease) (*Sandbox, error) {
	id, err := NewID()
	if err != nil {
		return nil, err
	}
	sandbox := Recorded(config, dependencies, Record{ID: id, Slot: slot.Index}, slot.Namespace)
	sandbox.lease = lease
	return sandbox, nil
}

// Recorded reconstructs the boot named by record after a crash, with no slot
// lease and no waiter. namespace is the recorded slot's network namespace name.
func Recorded(config Config, dependencies Dependencies, record Record, namespace string) *Sandbox {
	config.DNS = slices.Clone(config.DNS)
	if config.Limits != nil {
		limits := *config.Limits
		config.Limits = &limits
	}
	return &Sandbox{
		config:       config,
		dependencies: dependencies,
		record:       record,
		slot:         Slot{Index: record.Slot, Namespace: namespace},
		directory:    NewRuntimeDirectory(config.Runtime, record.ID),
	}
}

// ID returns the boot identity used by the device worker to reject stale exits.
func (s *Sandbox) ID() ID {
	return s.record.ID
}

// Start starts the boot on working with base as the lower filesystem. It writes
// the record before acquiring resources. Failure leaves partial resources for
// Close. The waiter belongs to Sandbox, not to the Start context.
func (s *Sandbox) Start(ctx context.Context, working Working, base string, boot runnerproto.ManagedBoot) error {
	if boot.BackendURL != s.config.BackendURL {
		return ErrAllowlist
	}
	record, err := s.record.MarshalJSON()
	if err != nil {
		return err
	}
	if err := artifacts.PublishBytes(
		ctx,
		filepath.Join(working.Directory(), "sandbox.json"),
		record,
		artifacts.Publication{Mode: artifacts.Replace, Durable: true},
	); err != nil {
		return err
	}
	system.FaultPoint("sandbox-record")
	if err := s.directory.Create(ctx); err != nil {
		return err
	}
	if err := system.Bind(ctx, base, s.directory.Base()); err != nil {
		return err
	}
	if err := system.RemountReadOnly(ctx, s.directory.Base()); err != nil {
		return err
	}
	system.FaultPoint("base-bound")
	if err := s.mountWorking(ctx, working); err != nil {
		return err
	}
	if err := system.Overlay(
		ctx,
		s.directory.Base(),
		s.directory.Volume(machinemanagerproto.VolumeSystem),
		s.directory.RootFS(),
	); err != nil {
		return err
	}
	if err := system.Tmpfs(ctx, s.directory.Credentials(), "size=1m,mode=0700"); err != nil {
		return err
	}
	if err := s.directory.WriteCredentials(ctx, boot, s.config.DNS); err != nil {
		return err
	}
	system.FaultPoint("credentials-written")
	if err := s.dependencies.Network.Attach(ctx, s.slot); err != nil {
		return err
	}
	system.FaultPoint("network-attached")
	if err := s.writeSpec(ctx); err != nil {
		return err
	}
	if err := s.startRuntime(ctx); err != nil {
		return err
	}
	system.FaultPoint("sandbox-started")
	s.startWaiting(ctx)
	return nil
}

// Exited waits for runsc wait to finish. Canceling this call retains the owned
// waiter for a later call; without a waiter it waits until ctx is canceled.
func (s *Sandbox) Exited(ctx context.Context) error {
	s.mu.Lock()
	waiter := s.waiter
	s.mu.Unlock()
	if waiter == nil {
		<-ctx.Done()
		return ctx.Err()
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-waiter.done:
		s.clearWaiter(waiter)
		return waiter.err
	}
}

// Status reports runsc's current view of this boot, including its absence.
func (s *Sandbox) Status(ctx context.Context) (status Status, found bool, err error) {
	return s.dependencies.Runsc.Status(ctx, s.record.ID)
}

// Pause suspends this boot for a checkpoint.
func (s *Sandbox) Pause(ctx context.Context) error {
	return s.dependencies.Runsc.Pause(ctx, s.record.ID)
}

// Resume resumes this boot after a checkpoint.
func (s *Sandbox) Resume(ctx context.Context) error {
	return s.dependencies.Runsc.Resume(ctx, s.record.ID)
}

// Capture freezes both filesystems, copies both images to copies and syncs
// the copies, then thaws both. Once entered, this synchronous window cannot be
// canceled midway. Deferred thaw covers every return and recovered panic. It
// returns each thaw that failed, and err when the copies were not made.
func (s *Sandbox) Capture(ctx context.Context, working Working, copies Images) (thawFailures []error, err error) {
	ctx = context.WithoutCancel(ctx)
	var frozen system.Frozen
	defer func() {
		if failure := recover(); failure != nil {
			if cause, ok := failure.(error); ok {
				err = fmt.Errorf("checkpoint panicked: %w", cause)
			} else {
				err = fmt.Errorf("checkpoint panicked: %v", failure)
			}
		}
		thawFailures = frozen.ThawAll(ctx)
	}()
	for _, volume := range []machinemanagerproto.Volume{machinemanagerproto.VolumeSystem, machinemanagerproto.VolumeHome} {
		if err := frozen.Freeze(ctx, s.directory.Volume(volume)); err != nil {
			return nil, err
		}
	}
	system.FaultPoint("frozen")
	for _, volume := range []machinemanagerproto.Volume{machinemanagerproto.VolumeSystem, machinemanagerproto.VolumeHome} {
		if err := s.captureImage(ctx, working.Image(volume), copies.Image(volume)); err != nil {
			return nil, err
		}
	}
	return nil, nil
}

// Grow extends the image without shrinking it, refreshes the boot's loop
// device and grows its mounted filesystem. bytes must be nonzero. The returned
// capacity is read from the filesystem after syncing the image.
func (s *Sandbox) Grow(
	ctx context.Context,
	working Working,
	volume machinemanagerproto.Volume,
	bytes uint64,
) (capacity uint64, err error) {
	if s.loops == nil {
		return 0, ErrNotGrowable
	}
	if err := volume.Validate(); err != nil {
		return 0, err
	}
	if bytes == 0 || bytes > math.MaxInt64 {
		return 0, fmt.Errorf("invalid Cloud image capacity: %d", bytes)
	}
	number := s.loops[0]
	if volume == machinemanagerproto.VolumeHome {
		number = s.loops[1]
	}
	image := working.Image(volume)
	file, err := os.OpenFile(image, os.O_WRONLY, 0)
	if err != nil {
		return 0, err
	}
	defer func() { err = errors.Join(err, file.Close()) }()
	info, err := file.Stat()
	if err != nil {
		return 0, err
	}
	if uint64(info.Size()) < bytes {
		if err := file.Truncate(int64(bytes)); err != nil {
			return 0, err
		}
	}
	system.FaultPoint("image-extended")
	if err := system.RefreshCapacity(ctx, number); err != nil {
		return 0, err
	}
	system.FaultPoint("capacity-refreshed")
	if err := s.dependencies.Disks.GrowMounted(ctx, system.LoopPath(number)); err != nil {
		return 0, err
	}
	if err := file.Sync(); err != nil {
		return 0, err
	}
	return s.dependencies.Disks.Capacity(ctx, image)
}

// Close stops the boot and releases resources in reverse acquisition order,
// tolerating absence. It thaws before signaling, joins the waiter, fences only
// with limits on, unmounts, detaches networking, removes credentials and the
// record, and releases the slot lease. Failure retains resources for retry.
func (s *Sandbox) Close(ctx context.Context, working Working) (err error) {
	if err := s.thawVolumes(ctx); err != nil {
		return err
	}
	if err := s.stopRuntime(ctx); err != nil {
		return err
	}
	s.mu.Lock()
	waiter := s.waiter
	s.mu.Unlock()
	if waiter != nil {
		// The runtime is stopped. Its wait outcome cannot alter cleanup, but the
		// process must finish before its resources disappear.
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-waiter.done:
		}
		s.clearWaiter(waiter)
	}
	if s.config.Limits != nil {
		if err := Fence(ctx, s.record.ID); err != nil {
			return err
		}
	}
	if err := s.unmountVolumes(ctx); err != nil {
		return err
	}
	if err := s.dependencies.Network.Detach(ctx, s.slot); err != nil {
		return err
	}
	if err := s.directory.Remove(ctx); err != nil {
		return err
	}
	if err := os.Remove(
		filepath.Join(working.Directory(), "sandbox.json"),
	); err != nil &&
		!errors.Is(err, os.ErrNotExist) {
		return err
	}
	directory, err := os.Open(working.Directory())
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, directory.Close()) }()
	if err := directory.Sync(); err != nil {
		return err
	}
	s.loops = nil
	if s.lease != nil {
		s.lease.Release()
		s.lease = nil
	}
	return nil
}

// StopWaiting cancels and joins the owned runsc wait command when shutdown
// follows a failed drain. It leaves the sandbox, its record and mounts intact
// for stop-post recovery. Losing the Go reference never performs this cleanup.
func (s *Sandbox) StopWaiting(_ context.Context) error {
	// Joining is cleanup: cancellation of this caller cannot abandon the child.
	s.mu.Lock()
	waiter := s.waiter
	s.mu.Unlock()
	if waiter != nil {
		waiter.cancel()
		<-waiter.done
		s.clearWaiter(waiter)
	}
	return nil
}

// mountImage keeps the loop descriptor until the mounted filesystem holds it.
func (s *Sandbox) mountImage(
	ctx context.Context,
	image string,
	volume machinemanagerproto.Volume,
) (number uint32, err error) {
	device, err := system.Attach(ctx, image)
	if err != nil {
		return 0, err
	}
	defer func() { err = errors.Join(err, device.Close()) }()
	if err := system.Ext4(ctx, device.Path(), s.directory.Volume(volume)); err != nil {
		return 0, err
	}
	return device.Number(), nil
}

// startRuntime owns the runtime log descriptor until runsc has inherited it.
func (s *Sandbox) startRuntime(ctx context.Context) (err error) {
	log, err := os.OpenFile(s.directory.Log(), os.O_APPEND|os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, log.Close()) }()
	return s.dependencies.Runsc.Start(ctx, s.record.ID, s.directory.Root(), log, s.directory.Log())
}

// startWaiting transfers the wait process to this boot rather than its request.
func (s *Sandbox) startWaiting(ctx context.Context) {
	ctx, cancel := context.WithCancel(context.WithoutCancel(ctx))
	waiter := &bootWaiter{cancel: cancel, done: make(chan struct{})}
	s.mu.Lock()
	s.waiter = waiter
	s.mu.Unlock()
	go func() {
		defer close(waiter.done)
		defer cancel()
		waiter.err = s.dependencies.Runsc.Wait(ctx, s.record.ID)
	}()
}

// clearWaiter consumes only the waiter the caller actually joined.
func (s *Sandbox) clearWaiter(waiter *bootWaiter) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.waiter == waiter {
		s.waiter = nil
	}
}

// captureImage copies a frozen boot image and syncs its checkpoint copy.
func (s *Sandbox) captureImage(ctx context.Context, source, destination string) (err error) {
	if err := s.dependencies.Disks.CloneSparse(ctx, source, destination); err != nil {
		return err
	}
	file, err := os.Open(destination)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, file.Close()) }()
	return file.Sync()
}

// stopRuntime resumes a paused boot, grants TERM its grace period, then deletes.
func (s *Sandbox) stopRuntime(ctx context.Context) error {
	runsc, id := s.dependencies.Runsc, s.record.ID
	status, found, err := runsc.Status(ctx, id)
	if err != nil {
		return err
	}
	if found && status != Stopped {
		if err := s.terminateRuntime(ctx, status); err != nil {
			return err
		}
	}
	_, found, err = runsc.Status(ctx, id)
	if err != nil {
		return err
	}
	if found {
		return runsc.Delete(ctx, id)
	}
	return nil
}

// mountWorking records loop numbers only after both working volumes have mounted.
func (s *Sandbox) mountWorking(ctx context.Context, working Working) error {
	var loops [2]uint32
	for i, volume := range []machinemanagerproto.Volume{machinemanagerproto.VolumeSystem, machinemanagerproto.VolumeHome} {
		image := working.Image(volume)
		output, err := s.dependencies.Tools.Output(ctx, system.E2fsck, []string{"-p", image}, 0)
		if err != nil {
			return err
		}
		if !output.Status.Exited() || (output.Status.ExitStatus() != 0 && output.Status.ExitStatus() != 1) {
			//nolint:staticcheck // User-visible text, kept byte for byte.
			return fmt.Errorf("Cloud %s filesystem needs recovery: %s", volume, output.Message())
		}
		number, err := s.mountImage(ctx, image, volume)
		if err != nil {
			return err
		}
		loops[i] = number
		system.FaultPoint("volume-mounted")
	}
	s.loops = &loops
	return nil
}

// thawVolumes thaws mounted volumes before the runtime is signaled.
func (s *Sandbox) thawVolumes(ctx context.Context) error {
	for _, volume := range []machinemanagerproto.Volume{machinemanagerproto.VolumeSystem, machinemanagerproto.VolumeHome} {
		path := s.directory.Volume(volume)
		mounted, _, err := system.MountRoot(ctx, path)
		if err != nil {
			return err
		}
		if mounted {
			if _, err := system.Thaw(ctx, path); err != nil {
				return err
			}
		}
	}
	return nil
}

// unmountVolumes releases mounts in the established cleanup order.
func (s *Sandbox) unmountVolumes(ctx context.Context) error {
	for _, name := range MountPoints() {
		path := filepath.Join(s.directory.Root(), name)
		mounted, _, err := system.MountRoot(ctx, path)
		if err != nil {
			return err
		}
		if !mounted {
			continue
		}
		if name == "system" || name == "home" {
			if _, err := system.Thaw(ctx, path); err != nil {
				return err
			}
		}
		if err := system.Unmount(ctx, path); err != nil {
			return err
		}
	}
	return nil
}

// terminateRuntime resumes a paused boot and waits through the original TERM grace period.
func (s *Sandbox) terminateRuntime(ctx context.Context, status Status) error {
	runsc, id := s.dependencies.Runsc, s.record.ID
	if status == Paused {
		if err := runsc.Resume(ctx, id); err != nil {
			return err
		}
	}
	signalled, err := runsc.Terminate(ctx, id)
	if err != nil {
		return err
	}
	if !signalled.Status.Exited() || signalled.Status.ExitStatus() != 0 {
		status, found, err := runsc.Status(ctx, id)
		if err != nil {
			return err
		}
		if !found || status != Stopped {
			//nolint:staticcheck // User-visible text, kept byte for byte.
			return fmt.Errorf("Cannot signal Cloud sandbox: %s", signalled.Message())
		}
	}
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		status, found, err := runsc.Status(ctx, id)
		if err != nil {
			return err
		}
		if !found || status != Running {
			break
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
	return nil
}

// writeSpec durably publishes the boot profile before starting runsc.
func (s *Sandbox) writeSpec(ctx context.Context) error {
	profile := Boot{Directory: s.directory, Namespace: s.slot.Namespace}
	if s.config.Limits != nil {
		profile.Cgroup = &Cgroup{Name: s.record.ID, Limits: *s.config.Limits}
	}
	spec, err := Spec(profile)
	if err != nil {
		return err
	}
	if err := artifacts.PublishBytes(
		ctx,
		s.directory.Config(),
		spec,
		artifacts.Publication{Mode: artifacts.Replace, Durable: true},
	); err != nil {
		return err
	}
	return nil
}
