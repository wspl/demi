package host

import (
	"context"
	"errors"
	"log/slog"
	"strings"
	"sync"

	"github.com/google/uuid"

	"github.com/wspl/demi/internal/runnerproto"
)

// ManagedVolume identifies a managed filesystem by its wire name and mount path.
type ManagedVolume struct {
	// Name identifies the volume to the manager.
	Name runnerproto.VolumeName
	// Mount is the filesystem's mount path on the Host.
	Mount string
}

// Volumes owns managed-volume capacity checks, growth requests and filesystem
// flushes. At most one growth request per volume is outstanding. Its owner
// drives Poll and Checked and supplies replies through Grown. Methods are safe
// for concurrent use. The owner calls Close before closing output.
type Volumes struct {
	life    *lifetime
	blocks  []ManagedVolume
	output  chan<- []byte
	syncs   chan struct{}
	checked chan struct{}
	// mu protects only the pending check/request map.
	mu      sync.Mutex
	pending map[runnerproto.VolumeName]string
	checks  chan volumeCheck
}
type volumeCheck struct {
	name   runnerproto.VolumeName
	wanted uint64
	grow   bool
}

// NewVolumes creates a volume owner and copies blocks. The caller owns output;
// the volume owner emits encoded volume_grow and sync_done frames there.
func NewVolumes(ctx context.Context, blocks []ManagedVolume, output chan<- []byte) *Volumes {
	return &Volumes{
		life:    newLifetime(ctx),
		blocks:  append([]ManagedVolume(nil), blocks...),
		output:  output,
		syncs:   make(chan struct{}, 4),
		checked: make(chan struct{}, 1),
		pending: make(map[runnerproto.VolumeName]string),
		checks:  make(chan volumeCheck, len(blocks)),
	}
}

// GrowthWanted returns a doubled capacity when free space is below the reserve,
// and false when no growth is needed. The reserve is a tenth of capacity, at least
// 256 MiB but capped at a quarter for small volumes. Growth beyond the protocol's
// safe integer limit fails; a zero-sized volume requests no growth.
func GrowthWanted(total, available uint64) (wanted uint64, grow bool, err error) {
	if total == 0 {
		return 0, false, nil
	}
	reserve := max(total/10, min(uint64(256*1024*1024), total/4))
	if available >= reserve {
		return 0, false, nil
	}
	if total > 9007199254740991/2 {
		return 0, false, errors.New("volume growth exceeds the protocol size limit")
	}
	return total * 2, true, nil
}

// Sync flushes the managed filesystems and emits sync_done. At most four flushes
// run concurrently; later requests wait with ctx. An admitted filesystem flush
// is joined even if the request is canceled. Paired Unix devices sync all
// filesystems; on Windows, Sync answers an error, since whole-filesystem sync is unavailable there.
func (v *Volumes) Sync(ctx context.Context, request runnerproto.Sync) error {
	ctx, leave, err := v.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	if err = admit(ctx, v.syncs); err != nil {
		return err
	}
	defer func() { <-v.syncs }()
	failure := syncFilesystems(v.blocks)
	var message *string
	if failure != nil {
		text := failure.Error()
		message = &text
	}
	return sendFrame(v.life.ctx, v.output, &runnerproto.SyncDone{ID: request.ID, Error: message})
}

// Poll starts capacity checks for volumes with no outstanding check or growth
// request. It does not wait for filesystem work; Close owns and joins that work.
func (v *Volumes) Poll() {
	_, leave, err := v.life.enter(v.life.ctx)
	if err != nil {
		return
	}
	defer leave()
	v.mu.Lock()
	var check []ManagedVolume
	for _, volume := range v.blocks {
		if _, pending := v.pending[volume.Name]; !pending {
			v.pending[volume.Name] = ""
			check = append(check, volume)
		}
	}
	v.mu.Unlock()
	for _, volume := range check {
		ctx, leave, err := v.life.enter(v.life.ctx)
		if err != nil {
			return
		}
		go func() {
			defer leave()
			total, available, err := volumeUsage(volume.Mount)
			var wanted uint64
			var grow bool
			if err == nil {
				wanted, grow, err = GrowthWanted(total, available)
			}
			if err != nil {
				slog.Warn("volume capacity check", "error", err)
			}
			select {
			case v.checks <- volumeCheck{name: volume.Name, wanted: wanted, grow: grow}:
			case <-ctx.Done():
			}
		}()
	}
}

// Checked waits for the next capacity check and sends a growth request when
// needed, recording its ID before sending. It returns false immediately when
// there are no pending checks. Filesystem check failures are logged and clear
// that volume's pending check; returned errors indicate cancellation or delivery.
func (v *Volumes) Checked(ctx context.Context) (bool, error) {
	ctx, leave, err := v.life.enter(ctx)
	if err != nil {
		return false, err
	}
	defer leave()
	if err := admit(ctx, v.checked); err != nil {
		return false, err
	}
	defer func() { <-v.checked }()
	pending := v.hasPendingCheck()
	if !pending {
		return false, nil
	}
	var result volumeCheck
	select {
	case <-ctx.Done():
		return false, ctx.Err()
	case result = <-v.checks:
	}
	if !result.grow {
		v.mu.Lock()
		delete(v.pending, result.name)
		v.mu.Unlock()
		return true, nil
	}
	id, err := uuid.NewRandom()
	if err != nil {
		v.mu.Lock()
		delete(v.pending, result.name)
		v.mu.Unlock()
		return true, err
	}
	requestID := strings.ReplaceAll(id.String(), "-", "")
	v.mu.Lock()
	v.pending[result.name] = requestID
	v.mu.Unlock()
	err = sendFrame(ctx, v.output, &runnerproto.VolumeGrow{ID: requestID, Volume: result.name, Bytes: result.wanted})
	if err != nil {
		v.mu.Lock()
		if v.pending[result.name] == requestID {
			delete(v.pending, result.name)
		}
		v.mu.Unlock()
	}
	return true, err
}

// Grown applies the matching volume growth response and clears its pending
// request, logging a manager failure. Unknown volumes or request IDs fail.
func (v *Volumes) Grown(request runnerproto.VolumeGrown) error {
	known := false
	for _, volume := range v.blocks {
		if volume.Name == request.Volume {
			known = true
			break
		}
	}
	if !known {
		return errors.New("unknown managed volume")
	}
	v.mu.Lock()
	id, ok := v.pending[request.Volume]
	if !ok || id == "" || id != request.ID {
		v.mu.Unlock()
		return errors.New("unexpected volume growth response")
	}
	delete(v.pending, request.Volume)
	v.mu.Unlock()
	if request.Error != nil {
		slog.Warn("volume growth failed", "volume", request.Volume, "bytes", request.Bytes, "error", *request.Error)
	}
	return nil
}

// Close cancels capacity checks and joins all checks and admitted flush work.
// It is idempotent and never closes output. Use a live cleanup context; after
// an interrupted wait another Close resumes waiting for owned work to end.
func (v *Volumes) Close(ctx context.Context) error {
	if err := v.life.close(ctx); err != nil {
		return err
	}
	v.mu.Lock()
	clear(v.pending)
	v.mu.Unlock()
	return nil
}

func (v *Volumes) hasPendingCheck() bool {
	v.mu.Lock()
	pending := false
	for _, id := range v.pending {
		if id == "" {
			pending = true
			break
		}
	}
	v.mu.Unlock()
	return pending
}
