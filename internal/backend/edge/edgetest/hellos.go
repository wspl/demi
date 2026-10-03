package edgetest

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"testing"

	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
)

// HoldHellos holds runner hellos at token lookup until the returned hold is
// released or t cleans up. Install hooks in usershard.Services.Hooks before
// starting the edge. Reuses the shard's hold so arrival counts, cancellation
// and cleanup have one owner. Bind-step holds use hooks.Hellos.Hold directly.
func HoldHellos(t testing.TB, hooks *usershardtest.Hooks) *usershardtest.StepHold {
	panic("not written: b-edge")
}
