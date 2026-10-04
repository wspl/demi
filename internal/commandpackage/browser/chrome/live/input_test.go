package live

import (
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
)

// Queue scenarios use no workers or clock waits and cost less than one second.
func TestQueuedWheelTurnsAndMovesReachPageAsNewestInput(t *testing.T) {
	wheel := func(y, delta float64, modifiers uint8) browserproto.LiveViewerMessage {
		return &browserproto.LiveViewerMessageWheel{Tab: "t1", X: 10, Y: y, DeltaY: delta, Modifiers: modifiers}
	}
	pointer := func(action browserproto.PointerAction, x float64, buttons uint8) browserproto.LiveViewerMessage {
		return &browserproto.LiveViewerMessagePointer{
			Tab:     "t1",
			Action:  action,
			X:       x,
			Y:       5,
			Button:  browserproto.PointerButtonNone,
			Buttons: buttons,
		}
	}
	queued := []browserproto.LiveViewerMessage{
		wheel(100, 40, 0), wheel(110, 40, 0), wheel(120, 30, 0), wheel(120, 40, 8),
		pointer(browserproto.PointerActionMove, 1, 0), pointer(browserproto.PointerActionMove, 2, 0),
		pointer(browserproto.PointerActionMove, 3, 1), pointer(browserproto.PointerActionDown, 3, 1),
	}
	items := make(chan inputItem, len(queued))
	for _, message := range queued {
		items <- inputItem{kind: inputMessage, message: message}
	}
	close(items)
	var got []browserproto.LiveViewerMessage
	item, ok := <-items
	for ok {
		message, ahead, hasAhead := newestInput(item.message, items)
		got = append(got, message)
		if hasAhead {
			item = ahead
		} else {
			item, ok = <-items
		}
	}
	want := []browserproto.LiveViewerMessage{wheel(120, 110, 0), queued[3], queued[5], queued[6], queued[7]}
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatalf("input (-want +got):\n%s", diff)
	}
}

func TestInputMergingPreservesOrderingBoundaries(t *testing.T) {
	first := &browserproto.LiveViewerMessageWheel{Tab: "t1", DeltaX: 2, DeltaY: 3}
	for _, boundary := range []inputItem{
		{kind: inputRelease},
		{kind: inputWatch},
		{kind: inputFinish},
		{kind: inputMessage, message: &browserproto.LiveViewerMessageWheel{Tab: "t2"}},
		{kind: inputMessage, message: &browserproto.LiveViewerMessageKey{Tab: "t1"}},
	} {
		items := make(chan inputItem, 2)
		items <- boundary
		items <- inputItem{kind: inputMessage, message: first}
		got, ahead, ok := newestInput(first, items)
		if got != first || !ok || ahead.kind != boundary.kind || ahead.message != boundary.message || len(items) != 1 {
			t.Fatalf(
				"got %v, %+v, %v, %d queued; want unchanged input, boundary %+v, and one queued",
				got,
				ahead,
				ok,
				len(items),
				boundary,
			)
		}
	}
	for _, next := range []*browserproto.LiveViewerMessagePointer{
		{Tab: "t2", Action: browserproto.PointerActionMove},
		{Tab: "t1", Action: browserproto.PointerActionMove, Modifiers: 8},
		{Tab: "t1", Action: browserproto.PointerActionUp},
	} {
		_, ok := mergeInput(
			&browserproto.LiveViewerMessagePointer{Tab: "t1", Action: browserproto.PointerActionMove},
			next,
		)
		if ok {
			t.Fatalf("merged ordering boundary %+v", next)
		}
	}
	got, ok := mergeInput(first, &browserproto.LiveViewerMessageWheel{Tab: "t1", X: 9, Y: 10, DeltaX: 4, DeltaY: -1})
	want := &browserproto.LiveViewerMessageWheel{Tab: "t1", X: 9, Y: 10, DeltaX: 6, DeltaY: 2}
	if diff := cmp.Diff(want, got); !ok || diff != "" {
		t.Fatalf("merged=%v (-want +got): %s", ok, diff)
	}
}
