package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// DebugOwner owns a tab's per-agent debugging sockets and bounded event history.
// The caller holds it in tab state, starts it lazily, and closes it at tab end.
type DebugOwner struct{}

// StartDebug starts debugging ownership for an existing target. No tab registry
// is consulted; the lifetime context ends with the tab. Close must join the owner.
func StartDebug(ctx context.Context, address string, id target.ID) *DebugOwner {
	panic("not written: k-chrome-cdp")
}

// Connect returns the caller's connection, making it if none exists.
func (d *DebugOwner) Connect(ctx context.Context, caller uint64) (*DebugHandle, error) {
	panic("not written: k-chrome-cdp")
}

// Detach closes the caller's connection and joins its cleanup.
func (d *DebugOwner) Detach(ctx context.Context, caller uint64) error {
	panic("not written: k-chrome-cdp")
}

// OtherCallers returns other agents whose connections are open.
func (d *DebugOwner) OtherCallers(caller *uint64) []uint64 { panic("not written: k-chrome-cdp") }

// Recorded returns a notification closed by the next recorded event. Obtain it
// before reading Events so an intervening event cannot be missed.
func (d *DebugOwner) Recorded() <-chan struct{} { panic("not written: k-chrome-cdp") }

// Events returns a filtered page of the tab's recorded debugging events.
func (d *DebugOwner) Events(ctx context.Context, query EventQuery) (EventPage, error) {
	panic("not written: k-chrome-cdp")
}

// Close closes every debugging connection and joins the owner.
func (d *DebugOwner) Close(ctx context.Context) error { panic("not written: k-chrome-cdp") }

// EventQuery describes a read of the bounded debugging history.
type EventQuery struct {
	After   *string
	Limit   uint
	Methods []string
	Target  string
}

// EventPage indicates when an empty read after a cursor can wait for new events.
type EventPage struct {
	Result browserop.CdpEventsResult
	Wait   bool
}

// DebugHandle addresses only the tab and descendants attached by its private pump.
type DebugHandle struct{}

// Send sends a pinned command to a connection-owned target handle.
// Driver setup may use denied public methods; ExecuteCommand enforces admission.
func (d *DebugHandle) Send(ctx context.Context, method string, params json.RawMessage, targetHandle string) (json.RawMessage, error) {
	panic("not written: k-chrome-cdp")
}

// Targets returns this connection's currently attached target handles.
func (d *DebugHandle) Targets(ctx context.Context) ([]string, error) {
	panic("not written: k-chrome-cdp")
}

// ExecuteCommand runs a raw CDP operation under the caller's existing tab
// admission. operation owns the bounded wait; ctx is the invocation cancellation
// context, so a bounded events wait can expire without detaching its caller.
func ExecuteCommand(ctx context.Context, operation *Operation, debug *DebugOwner, caller uint64, tab browserop.TabID, command browserop.Operation) (json.RawMessage, error) {
	panic("not written: k-chrome-cdp")
}

// Capabilities reports the WebMCP and CDP families offered by this document.
func Capabilities(ctx context.Context, executor Executor) ([]browserop.Capability, error) {
	panic("not written: k-chrome-cdp")
}
