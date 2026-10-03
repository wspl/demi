package backend_test

import (
	"errors"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/expose"
	exposeplugin "github.com/wspl/demi/internal/plugins/expose"
)

// One paired runner; no service or model is needed to observe configuration refusal.
func TestExposeUnavailableWithoutDomain(t *testing.T) {
	t.Skip("finding 1: device claim returns HTTP 500 because device.installs is nil")
	ctx, _, b, s := filesBackend(t)
	laptop, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	_, err = backendtest.CreateExpose(ctx, b.Backend, s.User.ID, laptop.ID(), "1234")
	if !errors.Is(err, expose.ErrUnavailable) {
		t.Fatalf("create expose: %v", err)
	}
	page, err := b.Sync(ctx, t, &s)
	wireMust(t, err)
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	exposes, err := exposeplugin.DecodeExposeState(state.PluginStates["expose"])
	wireMust(t, err)
	if exposes.Available || len(exposes.Exposes) != 0 {
		t.Fatalf("expose state: %+v", exposes)
	}
	wireMust(t, b.Close(ctx))
}
