package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"
)

// Executor executes typed cdproto commands. Use protocol.WithExecutor to call
// cdproto parameter Do methods; implementations write JSON through contract.EncodeJSON.
type Executor = protocol.Executor

// MessageLimit is Rust's maximum inbound WebSocket message size.
const MessageLimit int64 = 16 * 1024 * 1024

// Connection owns a browser WebSocket, bounded queues and joined event pumps.
type Connection struct{}

// Dial connects to Chrome. The lifetime context owns the transport workers;
// the caller must Close the connection even after that context ends.
func Dial(ctx context.Context, address string) (*Connection, error) {
	panic("not written: k-chrome-cdp")
}

// Execute sends a browser-level command and decodes its reply into result.
func (c *Connection) Execute(ctx context.Context, method string, params, result any) error {
	panic("not written: k-chrome-cdp")
}

// Attach attaches a flattened target session and tracks its renderer descendants.
func (c *Connection) Attach(ctx context.Context, id target.ID) (*Session, error) {
	panic("not written: k-chrome-cdp")
}

// Subscribe registers before the next command. Empty methods selects all events.
// The caller closes the subscription; overflow is reported by Next.
func (c *Connection) Subscribe(methods ...string) (*Subscription, error) {
	panic("not written: k-chrome-cdp")
}

// Done closes when the connection ends; Err retains its transport failure.
func (c *Connection) Done() <-chan struct{} { panic("not written: k-chrome-cdp") }

// Err reports the transport failure, or nil for explicit closure.
func (c *Connection) Err() error { panic("not written: k-chrome-cdp") }

// Close closes the socket and joins every connection worker.
func (c *Connection) Close(ctx context.Context) error { panic("not written: k-chrome-cdp") }

// Session is a target executor on its owning connection. A detached session
// cannot send commands; callers never supply session IDs to raw command APIs.
type Session struct{}

// Execute sends a typed command within this session.
func (s *Session) Execute(ctx context.Context, method string, params, result any) error {
	panic("not written: k-chrome-cdp")
}

// TargetID identifies the Chrome target, without a conversation tab number.
func (s *Session) TargetID() target.ID { panic("not written: k-chrome-cdp") }

// ID identifies the flattened CDP session for internal event routing.
func (s *Session) ID() target.SessionID { panic("not written: k-chrome-cdp") }

// Related returns an attached descendant, or nil when none is attached.
func (s *Session) Related(ctx context.Context, id target.ID) (FrameTarget, error) {
	panic("not written: k-chrome-cdp")
}

// Subscribe registers for this session's events before the next command.
func (s *Session) Subscribe(methods ...string) (*Subscription, error) {
	panic("not written: k-chrome-cdp")
}

// Close detaches descendants before the parent and releases local routing.
func (s *Session) Close(ctx context.Context) error { panic("not written: k-chrome-cdp") }

// Event preserves a validated vendor envelope and its original session identity.
// Params is decoded with cdproto's event types by DecodeEvent.
type Event struct {
	Method    string
	Params    json.RawMessage
	SessionID target.SessionID
}

// DecodeEvent decodes the vendor payload using the method's cdproto event type.
func DecodeEvent(event Event) (any, error) { panic("not written: k-chrome-cdp") }

// Subscription is a bounded event stream. A slow reader does not block commands.
type Subscription struct{}

// Next waits for an event, explicit loss notification, or terminal error.
func (s *Subscription) Next(ctx context.Context) (Event, error) { panic("not written: k-chrome-cdp") }

// Close unregisters the stream and releases its queued events.
func (s *Subscription) Close() { panic("not written: k-chrome-cdp") }

// EventLoss reports dropped events so registries reconcile and logs mark gaps.
type EventLoss struct{ Count uint64 }

// Error describes the lost events.
func (e *EventLoss) Error() string { panic("not written: k-chrome-cdp") }
