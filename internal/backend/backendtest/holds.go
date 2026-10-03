package backendtest

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

// CommitHold holds checkpoint commits; Release or test cleanup releases them.
type CommitHold = databasetest.CommitHold

// StepHold holds a flow at one step and counts arrivals, including canceled ones.
type StepHold = usershardtest.StepHold

// HelloStep identifies the token lookup or shard bind in a runner's hello.
type HelloStep = usershard.HelloStep

const (
	// HelloTokenLookup holds the edge's lookup while it watches socket closure.
	HelloTokenLookup = usershard.HelloTokenLookup
	// HelloBind holds the shard's binding of an accepted socket.
	HelloBind = usershard.HelloBind
)

// SyncStep identifies where a page's synchronization flow can be held.
type SyncStep = usershard.SyncStep

const (
	// SyncSnapshot holds a read snapshot before it is sent.
	SyncSnapshot = usershard.SyncSnapshot
	// SyncChanges holds a woken channel before it takes and reads changed parts.
	SyncChanges = usershard.SyncChanges
)

// HoldCommits holds every checkpoint commit from now until Release or cleanup.
func HoldCommits(t testing.TB, b *backend.Backend) *CommitHold { panic("not written: b-backend") }

// HoldHellos holds every runner hello at step until Release or cleanup.
// b must have been started by this package's Start or Harness.Start.
func HoldHellos(t testing.TB, b *backend.Backend, step HelloStep) *StepHold {
	panic("not written: b-backend")
}

// HoldSync holds every page channel at step until Release or cleanup.
// b must have been started by this package's Start or Harness.Start.
func HoldSync(t testing.TB, b *backend.Backend, step SyncStep) *StepHold {
	panic("not written: b-backend")
}

// FileGate returns the user's conversation file gate. A lease is activity;
// transitions reserve the gate and waiting entrants expose held operations.
func FileGate(ctx context.Context, b *backend.Backend, user webapi.UserID, conversation webapi.ConversationID) (*gates.Activity, error) {
	panic("not written: b-backend")
}

// RunRetention runs the user's retention pass and answers when it ends.
func RunRetention(ctx context.Context, b *backend.Backend, user webapi.UserID) error {
	panic("not written: b-backend")
}

// CreateExpose creates an hour-long expose as the expose plugin does, without
// an agent turn. The address is validated at entry.
func CreateExpose(ctx context.Context, b *backend.Backend, user webapi.UserID, device webapi.DeviceID, address string) (expose.Expose, error) {
	panic("not written: b-backend")
}

// Start installs flow holds before starting b and registers test cleanup that
// releases holds and joins the backend. Existing Config.Hooks are preserved.
func Start(ctx context.Context, t testing.TB, config backend.Config) (*backend.Backend, error) {
	panic("not written: b-backend")
}
