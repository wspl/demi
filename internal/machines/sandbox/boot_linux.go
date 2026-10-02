//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"net/netip"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// Config supplies trusted manager configuration. Limits is nil only when the
// operator explicitly disables resource limits; there is no fallback mode.
type Config struct {
	Runtime    string
	BackendURL runnerwire.BackendURL
	DNS        []netip.Addr
	Limits     *Limits
}

// Slot names a network allocation made by the manager, without choosing policy.
type Slot struct {
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
	Tools   *system.Tools
	Runsc   *Runsc
	Network Network
	Disks   Disks
}

// Images supplies paths for the two volumes. Storage owns image naming; the
// manager adapts its image pair to this boundary without copying that rule.
type Images interface {
	Image(volume machinewire.Volume) string
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
type Sandbox struct{}

// New creates a boot on slot and takes ownership of lease on success. On an
// error the caller still owns lease. Config and slot are copied.
func New(config Config, dependencies Dependencies, slot Slot, lease Lease) (*Sandbox, error) {
	panic("not written: m-sandbox")
}

// Recorded reconstructs the boot named by record after a crash, with no slot
// lease and no waiter. namespace is the recorded slot's network namespace name.
func Recorded(config Config, dependencies Dependencies, record Record, namespace string) *Sandbox {
	panic("not written: m-sandbox")
}

// ID returns the boot identity used by the device worker to reject stale exits.
func (s *Sandbox) ID() ID { panic("not written: m-sandbox") }

// Start starts the boot on working with base as the lower filesystem. It writes
// the record before acquiring resources. Failure leaves partial resources for
// Close. The waiter belongs to Sandbox, not to the Start context.
func (s *Sandbox) Start(ctx context.Context, working Working, base string, boot runnerwire.ManagedBoot) error {
	panic("not written: m-sandbox")
}

// Exited waits for runsc wait to finish. Canceling this call retains the owned
// waiter for a later call; without a waiter it waits until ctx is canceled.
func (s *Sandbox) Exited(ctx context.Context) error { panic("not written: m-sandbox") }

// Status reports runsc's current view of this boot, including its absence.
func (s *Sandbox) Status(ctx context.Context) (status Status, found bool, err error) {
	panic("not written: m-sandbox")
}

// Pause suspends this boot for a checkpoint.
func (s *Sandbox) Pause(ctx context.Context) error { panic("not written: m-sandbox") }

// Resume resumes this boot after a checkpoint.
func (s *Sandbox) Resume(ctx context.Context) error { panic("not written: m-sandbox") }

// Capture is what a checkpoint's frozen window produced: whether the copies
// were made, and each thaw that failed. Copied is nil on successful copying.
type Capture struct {
	Copied     error
	ThawErrors []error
}

// Capture freezes both filesystems, copies both images to copies and syncs
// the copies, then thaws both. Once entered, this synchronous window cannot be
// canceled midway. Deferred thaw covers every return and recovered panic.
func (s *Sandbox) Capture(ctx context.Context, working Working, copies Images) Capture {
	panic("not written: m-sandbox")
}

// Grow extends the image without shrinking it, refreshes the boot's loop
// device and grows its mounted filesystem. bytes must be nonzero. The returned
// capacity is read from the filesystem after syncing the image.
func (s *Sandbox) Grow(ctx context.Context, working Working, volume machinewire.Volume, bytes uint64) (uint64, error) {
	panic("not written: m-sandbox")
}

// Close stops the boot and releases resources in reverse acquisition order,
// tolerating absence. It thaws before signaling, joins the waiter, fences only
// with limits on, unmounts, detaches networking, removes credentials and the
// record, and releases the slot lease. Failure retains resources for retry.
func (s *Sandbox) Close(ctx context.Context, working Working) error { panic("not written: m-sandbox") }

// StopWaiting cancels and joins the owned runsc wait command when shutdown
// follows a failed drain. It leaves the sandbox, its record and mounts intact
// for stop-post recovery. Losing the Go reference never performs this cleanup.
func (s *Sandbox) StopWaiting(ctx context.Context) error { panic("not written: m-sandbox") }
