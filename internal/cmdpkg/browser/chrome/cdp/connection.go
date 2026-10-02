package cdp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"

	"github.com/chromedp/cdproto"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"
	"github.com/coder/websocket"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/go-json-experiment/json/jsontext"
)

// Executor executes typed cdproto commands. Use protocol.WithExecutor to call
// cdproto parameter Do methods. CDP values use cdproto's own JSON marshalers;
// contract.EncodeJSON is reserved for Demi wire values.
type Executor = protocol.Executor

// MessageLimit is Rust's maximum inbound WebSocket message size.
const MessageLimit int64 = 16 * 1024 * 1024

// Connection owns a browser WebSocket, bounded queues and joined event pumps.
type Connection struct {
	socket        *websocket.Conn
	ctx           context.Context
	cancel        context.CancelFunc
	done          chan struct{}
	outbound      chan []byte
	mu            sync.Mutex
	failure       error
	next          int64
	pending       map[int64]pendingCall
	sessions      map[target.SessionID]*Session
	subscriptions map[*Subscription]struct{}
	// record is installed before debugging setup; it never performs IO.
	record   func(Event, *Connection) error
	validate bool
}

type pendingCall struct {
	reply   chan commandReply
	method  string
	session target.SessionID
}
type commandReply struct {
	data json.RawMessage
	err  error
}

// Dial connects to Chrome. The lifetime context owns the transport workers;
// the caller must Close the connection even after that context ends.
func Dial(ctx context.Context, address string) (*Connection, error) {
	return dial(ctx, ctx, address, false, nil)
}

// dial separates Chrome connection setup cancellation from its owning lifetime.
func dial(ctx, owner context.Context, address string, validate bool, record func(Event, *Connection) error) (*Connection, error) {
	socket, response, err := websocket.Dial(ctx, address, nil)
	if err != nil {
		if response != nil && response.Body != nil {
			_ = response.Body.Close()
		} // Failed handshake body owns no useful data.
		return nil, &BrowserError{Kind: KindConnection, Cause: err}
	}
	socket.SetReadLimit(MessageLimit)
	lifetime, cancel := context.WithCancel(owner)
	c := &Connection{socket: socket, validate: validate, record: record, ctx: lifetime, cancel: cancel, done: make(chan struct{}), outbound: make(chan []byte, 16), pending: make(map[int64]pendingCall), sessions: make(map[target.SessionID]*Session), subscriptions: make(map[*Subscription]struct{})}
	go c.pump()
	return c, nil
}

// pump owns and joins the Chrome socket reader and writer.
func (c *Connection) pump() {
	written := make(chan struct{})
	go func() {
		defer close(written)
		for {
			select {
			case <-c.ctx.Done():
				return
			case data := <-c.outbound:
				if err := c.socket.Write(c.ctx, websocket.MessageText, data); err != nil {
					c.fail(&BrowserError{Kind: KindConnection, Cause: err})
					return
				}
			}
		}
	}()
	c.read()
	c.cancel()
	_ = c.socket.CloseNow() // Reader or cancellation already determines the terminal cause.
	<-written
	close(c.done)
}

// fail records the first Chrome transport failure and wakes every blocked call.
func (c *Connection) fail(err error) {
	c.mu.Lock()
	if c.ctx.Err() == nil && c.failure == nil {
		c.failure = err
	}
	c.mu.Unlock()
	c.cancel()
}

// read routes Chrome replies and events without waiting for a subscriber.
func (c *Connection) read() {
	for {
		kind, data, err := c.socket.Read(c.ctx)
		if err != nil {
			c.fail(&BrowserError{Kind: KindConnection, Cause: err})
			return
		}
		if kind != websocket.MessageText {
			c.fail(&BrowserError{Kind: KindCDP, Message: "malformed CDP response envelope"})
			return
		}
		var message cdproto.Message
		if err := jsonv2.Unmarshal(data, &message); err != nil {
			c.fail(&BrowserError{Kind: KindCDP, Cause: err})
			return
		}
		if message.ID != 0 {
			c.mu.Lock()
			pending, found := c.pending[message.ID]
			delete(c.pending, message.ID)
			c.mu.Unlock()
			if !found {
				continue
			} // The waiting command was cancelled.
			reply := commandReply{data: json.RawMessage(message.Result)}
			if message.SessionID != pending.session || (message.Error == nil) == (message.Result == nil) {
				reply.err = &BrowserError{Kind: KindCDP, Message: "malformed CDP response envelope"}
			} else if message.Error != nil {
				reply.err = &ProtocolError{Code: message.Error.Code, Message: message.Error.Message}
				// Chrome supplies only this message for an absent flattened session;
				// the Rust boundary performs the same protocol-specific classification.
				if message.Error.Message == "Session with given id not found." {
					reply.err = &BrowserError{Kind: KindTabNotFound, Cause: reply.err}
				}
			} else if c.validate {
				reply.err = validatePinned(pending.method, "returns", reply.data)
			}
			if pending.reply != nil {
				pending.reply <- reply
			} else if reply.err != nil {
				c.fail(reply.err)
				return
			}
			continue
		}
		if message.Method == "" || message.Params == nil || message.Error != nil || message.Result != nil {
			c.fail(&BrowserError{Kind: KindCDP, Message: "malformed CDP response envelope"})
			return
		}
		event := Event{Method: string(message.Method), Params: json.RawMessage(message.Params), SessionID: message.SessionID}
		if c.validate {
			if err := validatePinned(event.Method, "event", event.Params); err != nil {
				c.fail(err)
				return
			}
		}
		if err := c.route(event); err != nil {
			c.fail(err)
			return
		}
		if c.record != nil {
			if err := c.record(event, c); err != nil {
				c.fail(err)
				return
			}
		}
		c.mu.Lock()
		for subscription := range c.subscriptions {
			if subscription.session == "" || c.descendantLocked(event.SessionID, subscription.session) {
				subscription.deliver(event)
			}
		}
		c.mu.Unlock()
	}
}

// route tracks Chrome's flattened children before exposing their events.
func (c *Connection) route(event Event) error {
	switch event.Method {
	case "Target.attachedToTarget":
		if !c.validate {
			if err := validateTyped(event.Method, "event", event.Params); err != nil {
				return err
			}
		}
		var attached target.EventAttachedToTarget
		if err := jsonv2.Unmarshal(event.Params, &attached); err != nil {
			return &BrowserError{Kind: KindCDP, Cause: err}
		}
		if attached.TargetInfo == nil || attached.SessionID == "" {
			return &BrowserError{Kind: KindCDP, Message: "malformed CDP attachment"}
		}
		c.mu.Lock()
		s := c.sessions[attached.SessionID]
		if s == nil {
			s = &Session{connection: c, id: attached.SessionID, target: attached.TargetInfo.TargetID, parent: event.SessionID}
			c.sessions[s.id] = s
		}
		c.mu.Unlock()
		if event.SessionID != "" {
			return c.submit(c.ctx, s.id, "Target.setAutoAttach", descendantParams(), nil)
		}
	case "Target.detachedFromTarget":
		if !c.validate {
			if err := validateTyped(event.Method, "event", event.Params); err != nil {
				return err
			}
		}
		var detached target.EventDetachedFromTarget
		if err := jsonv2.Unmarshal(event.Params, &detached); err != nil {
			return &BrowserError{Kind: KindCDP, Cause: err}
		}
		c.mu.Lock()
		var removed []target.SessionID
		for id, s := range c.sessions {
			if c.descendantLocked(id, detached.SessionID) {
				s.detached = true
				removed = append(removed, id)
			}
		}
		for _, id := range removed {
			delete(c.sessions, id)
		}
		c.mu.Unlock()
	}
	return nil
}

// descendantLocked follows Chrome session ancestry while connection.mu is held.
func (c *Connection) descendantLocked(id, parent target.SessionID) bool {
	for id != "" {
		if id == parent {
			return true
		}
		s := c.sessions[id]
		if s == nil {
			return false
		}
		id = s.parent
	}
	return false
}

// submit queues a CDP command; the bounded writer owns all socket writes.
func (c *Connection) submit(ctx context.Context, session target.SessionID, method string, params any, reply chan commandReply) error {
	if params == nil {
		params = struct{}{}
	} // cdproto uses nil for parameterless commands.
	data, err := jsonv2.Marshal(params)
	if err != nil {
		return &BrowserError{Kind: KindCDP, Cause: err}
	}
	if c.validate {
		if err := validatePinned(method, "params", data); err != nil {
			return err
		}
	}
	c.mu.Lock()
	c.next++
	id := c.next
	c.pending[id] = pendingCall{reply: reply, method: method, session: session}
	c.mu.Unlock()
	message, err := jsonv2.Marshal(struct {
		ID        int64            `json:"id"`
		SessionID target.SessionID `json:"sessionId,omitzero"`
		Method    string           `json:"method"`
		Params    jsontext.Value   `json:"params"`
	}{id, session, method, jsontext.Value(data)})
	if err == nil {
		select {
		case c.outbound <- message:
			return nil
		case <-ctx.Done():
			err = ctx.Err()
		case <-c.ctx.Done():
			err = c.closedError()
		}
	}
	c.mu.Lock()
	delete(c.pending, id)
	c.mu.Unlock()
	return err
}

// execute waits for one Chrome command reply, removing abandoned requests.
func (c *Connection) execute(ctx context.Context, session target.SessionID, method string, params, result any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if c.ctx.Err() != nil {
		return c.closedError()
	}
	reply := make(chan commandReply, 1)
	if err := c.submit(ctx, session, method, params, reply); err != nil {
		return err
	}
	defer func() {
		c.mu.Lock()
		for id, pending := range c.pending {
			if pending.reply == reply {
				delete(c.pending, id)
				break
			}
		}
		c.mu.Unlock()
	}()
	select {
	case response := <-reply:
		if response.err != nil {
			return response.err
		}
		if result == nil {
			return nil
		}
		if !c.validate {
			if err := validateTyped(method, "returns", response.data); err != nil {
				return err
			}
		}
		if raw, ok := result.(*json.RawMessage); ok {
			*raw = response.data
			return nil
		}
		if err := jsonv2.Unmarshal(response.data, result); err != nil {
			return &BrowserError{Kind: KindCDP, Cause: err}
		}
		return nil
	case <-ctx.Done():
		return ctx.Err()
	case <-c.ctx.Done():
		return c.closedError()
	}
}

// Execute sends a browser-level command and decodes its reply into result.
func (c *Connection) Execute(ctx context.Context, method string, params, result any) error {
	return c.execute(ctx, "", method, params, result)
}

// descendantParams selects only the tab's renderer and worker descendants.
func descendantParams() *target.SetAutoAttachParams {
	return target.SetAutoAttach(true, false).WithFlatten(true).WithFilter(target.Filter{{Type: "iframe"}, {Type: "worker"}, {Type: "shared_worker"}, {Type: "service_worker"}, {Exclude: true}})
}

// Attach attaches a flattened target session and tracks its renderer descendants.
func (c *Connection) Attach(ctx context.Context, id target.ID) (*Session, error) {
	var attached target.AttachToTargetReturns
	if err := c.Execute(ctx, "Target.attachToTarget", target.AttachToTarget(id).WithFlatten(true), &attached); err != nil {
		return nil, err
	}
	c.mu.Lock()
	s := c.sessions[attached.SessionID]
	if s == nil {
		s = &Session{connection: c, id: attached.SessionID, target: id}
		c.sessions[s.id] = s
	}
	c.mu.Unlock()
	if err := s.Execute(ctx, "Target.setAutoAttach", descendantParams(), nil); err != nil {
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), ControlTimeout)
		defer cancel()
		return nil, AfterCleanup(err, s.Close(cleanup))
	}
	return s, nil
}

// Subscribe registers before the next command. Empty methods selects all events.
// The caller closes the subscription; overflow is reported by Next.
func (c *Connection) Subscribe(methods ...string) (*Subscription, error) {
	return c.subscribe("", methods)
}

// subscribe installs a bounded Chrome event listener without a worker goroutine.
func (c *Connection) subscribe(session target.SessionID, methods []string) (*Subscription, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.ctx.Err() != nil {
		return nil, &BrowserError{Kind: KindClosed}
	}
	s := &Subscription{connection: c, session: session, methods: append([]string(nil), methods...), wake: make(chan struct{}, 1)}
	c.subscriptions[s] = struct{}{}
	return s, nil
}

// Done closes when the connection ends; Err retains its transport failure.
func (c *Connection) Done() <-chan struct{} { return c.done }

// Err reports the transport failure, or nil for explicit closure.
func (c *Connection) Err() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.failure
}

// closedError translates the terminal browser connection state.
func (c *Connection) closedError() error {
	if err := c.Err(); err != nil {
		return err
	}
	return &BrowserError{Kind: KindClosed}
}

// Close closes the socket and joins every connection worker.
func (c *Connection) Close(ctx context.Context) error {
	c.cancel()
	_ = c.socket.CloseNow() // Closing an already failed socket is complete.
	select {
	case <-c.done:
		return nil
	case <-ctx.Done():
		<-c.done
		return ctx.Err()
	}
}

// Session is a target executor on its owning connection. A detached session
// cannot send commands; callers never supply session IDs to raw command APIs.
type Session struct {
	connection *Connection
	id         target.SessionID
	target     target.ID
	parent     target.SessionID
	detached   bool // connection.mu
}

// Execute sends a typed command within this session.
func (s *Session) Execute(ctx context.Context, method string, params, result any) error {
	s.connection.mu.Lock()
	detached := s.detached
	s.connection.mu.Unlock()
	if detached {
		return &BrowserError{Kind: KindTabNotFound}
	}
	return s.connection.execute(ctx, s.id, method, params, result)
}

// TargetID identifies the Chrome target, without a conversation tab number.
func (s *Session) TargetID() target.ID { return s.target }

// ID identifies the flattened CDP session for internal event routing.
func (s *Session) ID() target.SessionID { return s.id }

// Related returns an attached descendant, or nil when none is attached.
func (s *Session) Related(ctx context.Context, id target.ID) (FrameTarget, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	c := s.connection
	c.mu.Lock()
	defer c.mu.Unlock()
	for _, child := range c.sessions {
		if !child.detached && child.target == id && c.descendantLocked(child.id, s.id) {
			return child, nil
		}
	}
	return nil, nil
}

// Subscribe registers for this session's events before the next command.
func (s *Session) Subscribe(methods ...string) (*Subscription, error) {
	return s.connection.subscribe(s.id, methods)
}

// Close detaches descendants before the parent and releases local routing.
func (s *Session) Close(ctx context.Context) error {
	c := s.connection
	c.mu.Lock()
	var children []*Session
	for _, child := range c.sessions {
		if child.parent == s.id && !child.detached {
			children = append(children, child)
		}
	}
	detached := s.detached
	c.mu.Unlock()
	var result error
	for _, child := range children {
		result = AfterCleanup(result, child.Close(ctx))
	}
	if detached {
		return result
	}
	err := c.execute(ctx, s.parent, "Target.detachFromTarget", target.DetachFromTarget().WithSessionID(s.id), nil)
	var chrome *ProtocolError
	if connectionLoss(err) || ErrorCode(err) == "browser_lost" || (errors.As(err, &chrome) && chrome.Code == -32602 && chrome.Message == "No session with given id") {
		err = nil
	}
	if err == nil {
		c.mu.Lock()
		s.detached = true
		c.mu.Unlock()
	}
	return AfterCleanup(result, err)
}

// Event preserves a validated vendor envelope and its original session identity.
// Params is decoded with cdproto's event types by DecodeEvent.
type Event struct {
	Method    string
	Params    json.RawMessage
	SessionID target.SessionID
}

// DecodeEvent decodes the vendor payload using the method's cdproto event type.
func DecodeEvent(event Event) (any, error) {
	if err := validateTyped(event.Method, "event", event.Params); err != nil {
		return nil, err
	}
	return cdproto.UnmarshalMessage(&cdproto.Message{Method: cdproto.MethodType(event.Method), Params: jsontext.Value(event.Params), SessionID: event.SessionID})
}

// Subscription is a bounded event stream. A slow reader does not block commands.
type Subscription struct {
	connection *Connection
	session    target.SessionID
	methods    []string
	mu         sync.Mutex
	queue      []Event
	lost       uint64
	closed     bool
	wake       chan struct{}
}

// deliver retains the newest Chrome events and explicitly counts overwritten ones.
func (s *Subscription) deliver(event Event) {
	if len(s.methods) > 0 {
		matched := false
		for _, method := range s.methods {
			if method == event.Method {
				matched = true
				break
			}
		}
		if !matched {
			return
		}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return
	}
	if len(s.queue) == 16 {
		s.queue = s.queue[1:]
		s.lost++
	}
	s.queue = append(s.queue, event)
	select {
	case s.wake <- struct{}{}:
	default:
	}
}

// Next waits for an event, explicit loss notification, or terminal error.
func (s *Subscription) Next(ctx context.Context) (Event, error) {
	for {
		s.mu.Lock()
		if s.lost > 0 {
			count := s.lost
			s.lost = 0
			s.mu.Unlock()
			return Event{}, &EventLoss{Count: count}
		}
		if len(s.queue) > 0 {
			event := s.queue[0]
			s.queue[0] = Event{}
			s.queue = s.queue[1:]
			s.mu.Unlock()
			return event, nil
		}
		closed := s.closed
		s.mu.Unlock()
		if closed {
			return Event{}, &BrowserError{Kind: KindClosed}
		}
		select {
		case <-ctx.Done():
			return Event{}, ctx.Err()
		case <-s.connection.ctx.Done():
			return Event{}, s.connection.closedError()
		case <-s.wake:
		}
	}
}

// Close unregisters the stream and releases its queued events.
func (s *Subscription) Close() {
	s.connection.mu.Lock()
	delete(s.connection.subscriptions, s)
	s.connection.mu.Unlock()
	s.mu.Lock()
	s.closed = true
	s.queue = nil
	s.mu.Unlock()
	select {
	case s.wake <- struct{}{}:
	default:
	}
}

// EventLoss reports dropped events so registries reconcile and logs mark gaps.
type EventLoss struct{ Count uint64 }

// Error describes the lost events.
func (e *EventLoss) Error() string {
	return fmt.Sprintf("CDP event subscription lost %d events", e.Count)
}
