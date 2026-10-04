package tabs

import (
	"context"
	"errors"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

// Loading reports whether the browser loads the tab's top-level page.
func (t *Tab) Loading() bool {
	return t.loading.Load()
}

// observeLoading subscribes before the tab admits operations and follows its main frame.
func (t *Tab) observeLoading(ctx context.Context) error {
	events, err := t.Subscribe("Page.frameStartedLoading", "Page.frameStoppedLoading")
	if err != nil {
		return err
	}
	tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, t.session))
	if err != nil {
		events.Close()
		return err
	}
	err = t.StartTask(func(ctx context.Context) {
		defer events.Close()
		t.followLoading(ctx, events, tree.Frame.ID)
	})
	if err != nil {
		events.Close()
	}
	return err
}

func (t *Tab) followLoading(ctx context.Context, events *cdp.Subscription, main protocol.FrameID) {
	for {
		raw, err := events.Next(ctx)
		if errors.Is(err, cdp.ErrEventsLost) {
			continue
		}
		if err != nil {
			return
		}
		decoded, err := cdp.DecodeEvent(raw)
		if err != nil {
			continue
		}
		var frame protocol.FrameID
		var loading bool
		switch event := decoded.(type) {
		case *page.EventFrameStartedLoading:
			frame = event.FrameID
			loading = true
		case *page.EventFrameStoppedLoading:
			frame = event.FrameID
		default:
			continue
		}
		if frame == main && t.loading.Swap(loading) != loading {
			t.environment.markChanged()
		}
	}
}
