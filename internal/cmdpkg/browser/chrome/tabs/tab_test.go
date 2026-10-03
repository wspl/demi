package tabs

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

// Chooser listeners must be available on the renderer supplied to page actions.
// One scripted socket, no Chrome or clock waits.
func TestPageExecutorSubscribesToChooserEvents(t *testing.T) {
	server := cdptest.NewServer(
		t,
		cdptest.Exchange{
			Method: "Target.attachToTarget",
			Params: target.AttachToTarget("tab").WithFlatten(true),
			Result: target.AttachToTargetReturns{SessionID: "main"},
		},
		cdptest.Exchange{
			Method:    "Target.setAutoAttach",
			SessionID: "main",
			Params: target.SetAutoAttach(true, false).
				WithFlatten(true).
				WithFilter(target.Filter{
					{Type: "iframe"},
					{Type: "worker"},
					{Type: "shared_worker"},
					{Type: "service_worker"},
					{Exclude: true},
				}),
		},
	)
	connection, err := cdp.Dial(t.Context(), server.Address())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := connection.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	session, err := connection.Attach(t.Context(), "tab")
	if err != nil {
		t.Fatal(err)
	}
	tab := &Tab{session: session, ctx: t.Context()}
	source, ok := tab.Page().(interface {
		Subscribe(...string) (*cdp.Subscription, error)
	})
	if !ok {
		t.Fatal("page executor cannot observe a file chooser")
	}
	events, err := source.Subscribe("Page.fileChooserOpened")
	if err != nil {
		t.Fatal(err)
	}
	defer events.Close()
	// An unrelated tab's chooser must not be delivered to this renderer.
	for _, id := range []target.SessionID{"other", "main"} {
		if err := server.Emit(
			t.Context(),
			cdp.Event{
				Method:    "Page.fileChooserOpened",
				SessionID: id,
				Params:    json.RawMessage(`{"frameId":"frame","mode":"selectSingle","backendNodeId":7}`),
			},
		); err != nil {
			t.Fatal(err)
		}
	}
	event, err := events.Next(t.Context())
	if err != nil || event.SessionID != "main" {
		t.Fatalf("chooser=%+v err=%v", event, err)
	}
}
