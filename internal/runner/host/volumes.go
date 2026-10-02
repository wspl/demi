package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// ManagedVolume identifies a managed filesystem by its wire name and mount path.
type ManagedVolume struct {
	// Name identifies the volume to the manager.
	Name runnerwire.VolumeName
	// Mount is the filesystem's mount path on the Host.
	Mount string
}

// Volumes owns managed-volume capacity checks, growth requests and filesystem
// flushes. At most one growth request per volume is outstanding. Its owner
// drives Poll and Checked and supplies replies through Grown. Methods are safe
// for concurrent use. The owner calls Close before closing output.
type Volumes struct{}

// NewVolumes creates a volume owner and copies blocks. The caller owns output;
// the volume owner emits encoded volume_grow and sync_done frames there.
func NewVolumes(ctx context.Context, blocks []ManagedVolume, output chan<- []byte) *Volumes {
	panic("not written: r-host")
}

// GrowthWanted returns a doubled capacity when free space is below the reserve,
// or nil when no growth is needed. The reserve is a tenth of capacity, at least
// 256 MiB but capped at a quarter for small volumes. Growth beyond the protocol's
// safe integer limit fails; a zero-sized volume requests no growth.
func GrowthWanted(total, available uint64) (*uint64, error) {
	panic("not written: r-host")
}

// Sync flushes the managed filesystems and emits sync_done. At most four flushes
// run concurrently; later requests wait with ctx. An admitted filesystem flush
// is joined even if the request is canceled. Paired Unix devices sync all
// filesystems; Windows answers an unsupported-operation error, matching Rust.
func (v *Volumes) Sync(ctx context.Context, request runnerwire.Sync) error {
	panic("not written: r-host")
}

// Poll starts capacity checks for volumes with no outstanding check or growth
// request. It does not wait for filesystem work; Close owns and joins that work.
func (v *Volumes) Poll() {
	panic("not written: r-host")
}

// Checked waits for the next capacity check and sends a growth request when
// needed, recording its ID before sending. It returns false immediately when
// there are no pending checks. Filesystem check failures are logged and clear
// that volume's pending check; returned errors indicate cancellation or delivery.
func (v *Volumes) Checked(ctx context.Context) (bool, error) {
	panic("not written: r-host")
}

// Grown applies the matching volume growth response and clears its pending
// request, logging a manager failure. Unknown volumes or request IDs fail.
func (v *Volumes) Grown(request runnerwire.VolumeGrown) error {
	panic("not written: r-host")
}

// Close cancels capacity checks and joins all checks and admitted flush work.
// It is idempotent and never closes output. Use a live cleanup context; after
// an interrupted wait another Close resumes waiting for owned work to end.
func (v *Volumes) Close(ctx context.Context) error {
	panic("not written: r-host")
}
