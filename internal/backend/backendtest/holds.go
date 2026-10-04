package backendtest

import (
	"context"
	"testing"
	"time"

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

// HelloStep identifies a step in a runner's hello where it can be held.
type HelloStep = usershard.HelloStep

// HelloBind holds the shard's binding of an accepted socket.
const HelloBind = usershard.HelloBind

// SyncStep identifies where a page's synchronization flow can be held.
type SyncStep = usershard.SyncStep

const (
	// SyncSnapshot holds a read snapshot before it is sent.
	SyncSnapshot = usershard.SyncSnapshot
	// SyncChanges holds a woken channel before it takes and reads changed parts.
	SyncChanges = usershard.SyncChanges
)

// HoldCommits holds every checkpoint commit from now until Release or cleanup.
func HoldCommits(t testing.TB, b *backend.Backend) *CommitHold {
	return databasetest.HoldCommits(t, b.Services().Conversations)
}

// HoldHellos holds every runner hello at step until Release or cleanup.
// b must have been started by this package's Start or Harness.Start.
func HoldHellos(t testing.TB, b *backend.Backend, step HelloStep) *StepHold {
	t.Helper()
	hooks, ok := b.Services().Hooks.(*fixtureHooks)
	if !ok {
		t.Fatal("HoldHellos requires backendtest.Start")
	}
	return hooks.hooks.Hellos.Hold(t, step)
}

// HoldSync holds every page channel at step until Release or cleanup.
// b must have been started by this package's Start or Harness.Start.
func HoldSync(t testing.TB, b *backend.Backend, step SyncStep) *StepHold {
	t.Helper()
	hooks, ok := b.Services().Hooks.(*fixtureHooks)
	if !ok {
		t.Fatal("HoldSync requires backendtest.Start")
	}
	return hooks.hooks.Syncs.Hold(t, step)
}

// FileGate returns the user's conversation file gate. A lease is activity;
// transitions reserve the gate and waiting entrants expose held operations.
func FileGate(
	ctx context.Context,
	b *backend.Backend,
	user webapi.UserID,
	conversation webapi.ConversationID,
) (*gates.Activity, error) {
	shard, err := b.Shards().Of(ctx, user)
	if err != nil {
		return nil, err
	}
	return shard.Conversations().Slot(conversation).FileGate(), nil
}

// RunRetention runs the user's retention pass and answers when it ends.
func RunRetention(ctx context.Context, b *backend.Backend, user webapi.UserID) error {
	shard, err := b.Shards().Of(ctx, user)
	if err != nil {
		return err
	}
	return shard.RetentionPass(ctx)
}

// CreateExpose creates an hour-long expose as the expose plugin does, without
// an agent turn. The address is validated at entry.
func CreateExpose(
	ctx context.Context,
	b *backend.Backend,
	user webapi.UserID,
	device webapi.DeviceID,
	address string,
) (expose.Expose, error) {
	parsed, err := webapi.ParseExposeAddress(address)
	if err != nil {
		return expose.Expose{}, err
	}
	shard, err := b.Shards().Of(ctx, user)
	if err != nil {
		return expose.Expose{}, err
	}
	return expose.Add(ctx, shard.ExposeShard(), device, parsed, time.Hour)
}

// Start installs flow holds before starting b and registers test cleanup that
// releases holds and joins the backend. Existing Config.Hooks are preserved.
func Start(ctx context.Context, t testing.TB, config backend.Config) (*backend.Backend, error) {
	t.Helper()
	config.Hooks = &fixtureHooks{previous: config.Hooks}
	b, err := backend.Start(ctx, config)
	if err != nil {
		return nil, err
	}
	t.Cleanup(func() {
		if err := b.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return b, nil
}

// fixtureHooks composes scenario holds with caller-supplied flow observations.
type fixtureHooks struct {
	hooks    usershardtest.Hooks
	previous usershard.FlowHooks
}

// Hello runs the fixture hold before forwarding the runner hello observation.
func (h *fixtureHooks) Hello(ctx context.Context, step usershard.HelloStep) error {
	if err := h.hooks.Hello(ctx, step); err != nil {
		return err
	}
	if h.previous != nil {
		return h.previous.Hello(ctx, step)
	}
	return nil
}

// Sync runs the fixture hold before forwarding the page synchronization observation.
func (h *fixtureHooks) Sync(ctx context.Context, step usershard.SyncStep) error {
	if err := h.hooks.Sync(ctx, step); err != nil {
		return err
	}
	if h.previous != nil {
		return h.previous.Sync(ctx, step)
	}
	return nil
}
