package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/webapi"
)

// Cloud is the user's one machine, made on first need, and its lifecycle work.
// Construct it with New and share its pointer. Its opaque state uses the shard
// mutex; methods acquire that mutex themselves and must be called without it
// held. No IO, callback or wait runs under that mutex.
type Cloud struct{}

// New creates the Cloud component using its owning shard's lifetime context
// and mutex. Request contexts never own transitions. The shard must call Close
// to join all work before releasing its dependencies, even if ctx is canceled.
func New(ctx context.Context, mu *sync.Mutex) *Cloud { panic("not written: b-cloud") }

// Runs reports whether device's machine runs, which a new expose on it needs.
func (c *Cloud) Runs(device webapi.DeviceID) bool { panic("not written: b-cloud") }

// Stop admits nothing more and cancels the running machine's schedules. It
// does not wait; a retirement already running finishes, and Close joins it.
func (c *Cloud) Stop() { panic("not written: b-cloud") }

// Admission holds the Cloud running. Each operation of a Host made under it
// takes PerOperation. The acquiring owner defers Release.
type Admission struct {
	// Device is the admitted Cloud's identity.
	Device webapi.DeviceID
	// PerOperation acquires a separate lease for each Host operation.
	PerOperation remotehost.Admission
}

// Release lets this admission go. It is idempotent and does not wait.
func (a *Admission) Release() { panic("not written: b-cloud") }

// Admit wakes a stopped Cloud, joins a boot under way, or waits for a running
// reset, holding nothing while it waits for that reset. A failed reset fails
// its waiters. Cancellation ends the wait, not the Cloud-owned transition.
// The caller must own device through shard and release the returned admission.
func Admit(ctx context.Context, shard CloudShard, device database.DeviceRecord) (*Admission, error) {
	panic("not written: b-cloud")
}

// Died handles an unsolicited sandbox exit. A running machine stops, its
// runner disconnects, its exposes end, and the death counts toward crash-loop
// protection, as one during boot does. Saving and resetting ignore the event.
func Died(ctx context.Context, shard CloudShard, device webapi.DeviceID) error {
	panic("not written: b-cloud")
}

// Close stops admission and schedules, joins owned work including a reset or
// transition under way, and saves and stops a running machine. A machine still
// held is left to the manager's reconcile when its client disconnects. The
// shard calls this with a usable cleanup context before disposing its state.
func Close(ctx context.Context, shard CloudShard) error { panic("not written: b-cloud") }
