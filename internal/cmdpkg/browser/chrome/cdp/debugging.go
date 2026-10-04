package cdp

import (
	"context"
	"encoding/json"
	"slices"
	"strings"
	"sync"

	"github.com/chromedp/cdproto/target"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
)

// DebugOwner owns a tab's per-agent debugging sockets and bounded event history.
// The caller holds it in tab state, starts it lazily, and closes it at tab end.
type DebugOwner struct {
	ctx         context.Context
	cancel      context.CancelFunc
	done        chan struct{}
	gate        chan struct{}
	address     string
	target      target.ID
	mu          sync.Mutex
	connections map[uint64]*DebugConnection
	buffer      *Buffer[browserproto.CDPEvent]
	recorded    chan struct{}
	cleanup     error
}

// StartDebug starts debugging ownership for an existing target. No tab registry
// is consulted; the lifetime context ends with the tab. Close must join the owner.
func StartDebug(ctx context.Context, address string, id target.ID) *DebugOwner {
	lifetime, cancel := context.WithCancel(ctx)
	d := &DebugOwner{
		ctx:         lifetime,
		cancel:      cancel,
		done:        make(chan struct{}),
		gate:        make(chan struct{}, 1),
		address:     address,
		target:      id,
		connections: map[uint64]*DebugConnection{},
		recorded:    make(chan struct{}),
	}
	go d.run()
	return d
}

// run owns terminal cleanup of every browser debugging connection.
func (d *DebugOwner) run() {
	defer close(d.done)
	<-d.ctx.Done()
	d.gate <- struct{}{}
	defer func() {
		<-d.gate
	}()
	d.mu.Lock()
	connections := d.connections
	d.connections = map[uint64]*DebugConnection{}
	d.buffer = nil
	close(d.recorded)
	d.recorded = make(chan struct{})
	d.mu.Unlock()
	var result error
	for _, handle := range connections {
		result = AfterCleanup(result, handle.close(context.Background()))
	}
	d.mu.Lock()
	d.cleanup = result
	d.mu.Unlock()
}

// acquire serializes debugging lifecycle operations without a mutex held across IO.
func (d *DebugOwner) acquire(ctx context.Context) error {
	select {
	case d.gate <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	case <-d.ctx.Done():
		return &BrowserError{Kind: KindClosed}
	}
}

// Connect returns the caller's connection, making it if none exists.
func (d *DebugOwner) Connect(ctx context.Context, caller uint64) (*DebugConnection, error) {
	if err := d.acquire(ctx); err != nil {
		return nil, err
	}
	defer func() {
		<-d.gate
	}()
	if d.ctx.Err() != nil {
		return nil, &BrowserError{Kind: KindClosed}
	}
	existing, err := d.existingConnection(caller)
	if err != nil {
		return nil, err
	}
	if existing != nil {
		if existing.connection.ctx.Err() != nil {
			return nil, &BrowserError{Kind: KindConnection, Message: "tab debugging connection ended"}
		}
		return existing, nil
	}
	setup, cancel := context.WithTimeout(ctx, ControlTimeout)
	defer cancel()
	callbackDone := make(chan struct{})
	stop := context.AfterFunc(d.ctx, func() {
		defer close(callbackDone)
		cancel()
	})
	defer func() {
		if !stop() {
			<-callbackDone
		}
	}()
	connection, err := dial(setup, d.ctx, d.address, true, d.record)
	if err == nil {
		var session *Session
		session, err = connection.Attach(setup, d.target)
		if err == nil {
			handle := &DebugConnection{connection: connection, main: session}
			d.mu.Lock()
			d.connections[caller] = handle
			d.mu.Unlock()
			return handle, nil
		}
		cleanup, release := context.WithTimeout(context.Background(), ControlTimeout)
		err = AfterCleanup(err, connection.Close(cleanup))
		release()
	}
	d.mu.Lock()
	if len(d.connections) == 0 {
		d.buffer = nil
	}
	d.mu.Unlock()
	return nil, err
}

// record appends validated Chrome events to the shared debugging generation.
func (d *DebugOwner) record(event Event, c *Connection) error {
	c.mu.Lock()
	session := c.sessions[event.SessionID]
	name := ""
	if session != nil && !session.detached {
		name = string(session.target)
		if session.target == d.target {
			name = "main"
		}
	}
	c.mu.Unlock()
	if name == "" {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.buffer == nil {
		return nil
	}
	if err := d.buffer.Push(
		browserproto.CDPEvent{Method: event.Method, Params: event.Params, Target: name},
	); err != nil {
		if gap := d.buffer.MarkGap(); gap != nil {
			return AfterCleanup(err, gap)
		}
	}
	close(d.recorded)
	d.recorded = make(chan struct{})
	return nil
}

// Detach closes the caller's connection and joins its cleanup.
func (d *DebugOwner) Detach(ctx context.Context, caller uint64) error {
	if err := d.acquire(ctx); err != nil {
		return err
	}
	defer func() {
		<-d.gate
	}()
	d.mu.Lock()
	handle := d.connections[caller]
	delete(d.connections, caller)
	if len(d.connections) == 0 {
		d.buffer = nil
	}
	d.mu.Unlock()
	if handle == nil {
		return nil
	}
	return handle.close(ctx)
}

// OtherCallers returns other agents whose connections are open.
func (d *DebugOwner) OtherCallers(caller *uint64) []uint64 {
	d.mu.Lock()
	defer d.mu.Unlock()
	result := []uint64{}
	for number, handle := range d.connections {
		if (caller == nil || *caller != number) && handle.connection.ctx.Err() == nil {
			result = append(result, number)
		}
	}
	slices.Sort(result)
	return result
}

// Recorded returns a notification closed by the next recorded event. Obtain it
// before reading Events so an intervening event cannot be missed.
func (d *DebugOwner) Recorded() <-chan struct{} {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.recorded
}

// Events returns a filtered page of the tab's recorded debugging events.
func (d *DebugOwner) Events(ctx context.Context, query EventQuery) (EventPage, error) {
	if err := ctx.Err(); err != nil {
		return EventPage{}, err
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	b := d.buffer
	if b == nil {
		return EventPage{}, &BrowserError{Kind: KindConnection, Message: "tab debugging connection ended"}
	}
	result := browserproto.CDPEventsResult{Events: []browserproto.CDPEvent{}, Cursor: b.Cursor(b.Next())}
	if query.After == nil {
		return EventPage{Result: result}, nil
	}
	position, err := b.Position(*query.After)
	if err != nil {
		return EventPage{}, err
	}
	for _, entry := range b.entries {
		if entry.Sequence < position || entry.Target != query.Target ||
			(query.Methods != nil && !slices.Contains(query.Methods, entry.Method)) {
			continue
		}
		if uint(len(result.Events)) == query.Limit {
			result.HasMore = true
			break
		}
		result.Events = append(result.Events, entry)
	}
	if result.HasMore && len(result.Events) > 0 {
		result.Cursor = b.Cursor(result.Events[len(result.Events)-1].Sequence + 1)
	}
	result.Truncated = b.TruncatedSince(position)
	return EventPage{Result: result, Wait: len(result.Events) == 0}, nil
}

// Close closes every debugging connection and joins the owner.
func (d *DebugOwner) Close(ctx context.Context) error {
	d.cancel()
	select {
	case <-d.done:
		d.mu.Lock()
		defer d.mu.Unlock()
		return d.cleanup
	case <-ctx.Done():
		<-d.done
		return ctx.Err()
	}
}

// EventQuery describes a read of the bounded debugging history.
type EventQuery struct {
	// After selects the cursor to read after.
	After *string
	// Limit bounds the returned event count.
	Limit uint
	// Methods filters the event names.
	Methods []string
	// Target selects the debugging target.
	Target string
}

// EventPage indicates when an empty read after a cursor can wait for new events.
type EventPage struct {
	// Result holds the bounded event page.
	Result browserproto.CDPEventsResult
	// Wait reports whether the caller can wait for more events.
	Wait bool
}

// DebugConnection is one agent's debugging connection to a tab and the descendants its private pump attached.
type DebugConnection struct {
	connection *Connection
	main       *Session
}

// close detaches Chrome debugging state and always joins the private socket.
func (d *DebugConnection) close(ctx context.Context) error {
	cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), ControlTimeout)
	defer cancel()
	err := d.main.Close(cleanup)
	return AfterCleanup(err, d.connection.Close(cleanup))
}

// Send sends a pinned command to a connection-owned target handle.
// Driver setup may use denied public methods; ExecuteCommand enforces admission.
func (d *DebugConnection) Send(
	ctx context.Context,
	method string,
	params json.RawMessage,
	targetHandle string,
) (json.RawMessage, error) {
	var selected Executor = d.main
	if targetHandle != "main" {
		related, err := d.main.Related(ctx, target.ID(targetHandle))
		if err != nil {
			return nil, err
		}
		if related == nil {
			return nil, &BrowserError{Kind: KindTargetNotFound}
		}
		selected = related
	}
	var result json.RawMessage
	err := selected.Execute(ctx, method, jsontext.Value(params), &result)
	return result, err
}

// Targets returns this connection's currently attached target handles.
func (d *DebugConnection) Targets(ctx context.Context) ([]string, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	c := d.connection
	c.mu.Lock()
	defer c.mu.Unlock()
	result := []string{}
	for _, s := range c.sessions {
		if s.detached || !c.descendantLocked(s.id, d.main.id) {
			continue
		}
		name := string(s.target)
		if s.id == d.main.id {
			name = "main"
		}
		result = append(result, name)
	}
	slices.Sort(result)
	return result, nil
}

var (
	deniedDomains = []string{"Target", "Browser", "SystemInfo", "Tethering", "HeadlessExperimental"}
	deniedMethods = []string{"Page.close", "Page.crash", "Page.setDownloadBehavior"}
)

// admit applies the public Chrome debugging method policy before opening a socket.
func admit(method string) error {
	domain, _, ok := strings.Cut(method, ".")
	if !ok {
		return &BrowserError{Kind: KindConfiguration, Message: "CDP method must be Domain.method"}
	}
	if slices.Contains(deniedDomains, domain) || slices.Contains(deniedMethods, method) {
		return &BrowserError{Kind: KindCDPMethodDenied, Message: method}
	}
	return nil
}

// ExecuteCommand runs a raw CDP operation under the caller's existing tab
// admission. operation owns the bounded wait; ctx is the invocation cancellation
// context, so a bounded events wait can expire without detaching its caller.
func ExecuteCommand(
	ctx context.Context,
	operation *Operation,
	debug *DebugOwner,
	caller uint64,
	tab browserproto.TabID,
	command browserproto.Operation,
) (json.RawMessage, error) {
	if _, ok := command.(*browserproto.CDPDetachInput); ok {
		return commandValue(browserproto.CDPDetachResult{Detached: tab}, debug.Detach(ctx, caller))
	}
	if input, ok := command.(*browserproto.CDPSendInput); ok {
		if err := admit(input.Method); err != nil {
			return nil, err
		}
		if err := validatePinned(input.Method, "params", []byte(input.Params)); err != nil {
			return nil, &BrowserError{Kind: KindConfiguration, Message: err.Error(), Cause: err}
		}
	}
	var connection *DebugConnection
	if err := operation.Run(ctx, func(work context.Context) error {
		var err error
		connection, err = debug.Connect(work, caller)
		return err
	}); err != nil {
		return nil, err
	}
	if input, ok := command.(*browserproto.CDPTargetsInput); ok {
		return debugTargets(ctx, operation, connection, input)
	}
	if input, ok := command.(*browserproto.CDPSendInput); ok {
		return debugSend(ctx, operation, connection, debug, caller, input)
	}
	if input, ok := command.(*browserproto.CDPEventsInput); ok {
		return debugEvents(ctx, operation, connection, debug, caller, input)
	}
	return nil, &BrowserError{Kind: KindConfiguration, Message: "CDP dispatch accepts only targets/send/events/detach"}
}

// commandValue encodes a browser result only after the command has succeeded.
func commandValue(value any, err error) (json.RawMessage, error) {
	if err != nil {
		return nil, err
	}
	return Value(value)
}

// Capabilities reports the WebMCP and CDP families offered by this document.
func Capabilities(ctx context.Context, executor Executor) ([]browserproto.Capability, error) {
	capability, err := webMCPCapability(ctx, executor)
	if err != nil {
		return nil, err
	}
	schema, err := Value(
		map[string]any{
			"deniedDomains": deniedDomains,
			"deniedMethods": deniedMethods,
			"help":          "demi browser cdp --help",
		},
	)
	if err != nil {
		return nil, err
	}
	return []browserproto.Capability{capability, {ID: "cdp", Available: true, Schema: &schema}}, nil
}

func debugTargets(
	ctx context.Context,
	operation *Operation,
	connection *DebugConnection,
	input *browserproto.CDPTargetsInput,
) (json.RawMessage, error) {
	var rows []browserproto.CDPTarget
	err := operation.Run(ctx, func(work context.Context) error {
		if _, err := connection.Send(work, "Runtime.getIsolateId", json.RawMessage(`{}`), "main"); err != nil {
			return err
		}
		ids, err := connection.Targets(work)
		if err != nil {
			return err
		}
		rows = []browserproto.CDPTarget{}
		for _, id := range ids {
			raw, err := connection.Send(work, "Target.getTargetInfo", json.RawMessage(`{}`), id)
			if ErrorCode(err) == "target_not_found" {
				continue
			}
			if err != nil {
				return err
			}
			var info target.GetTargetInfoReturns
			if err := jsonv2.Unmarshal(raw, &info); err != nil {
				return err
			}
			rows = append(rows, browserproto.CDPTarget{ID: id, Kind: info.TargetInfo.Type, URL: info.TargetInfo.URL})
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	offset, limit := uint(0), uint(browserproto.DefaultNodes)
	if input.Offset != nil {
		offset = *input.Offset
	}
	if input.Limit != nil {
		limit = *input.Limit
	}
	start := min(offset, uint(len(rows)))
	end := start + min(limit, uint(len(rows))-start)
	return Value(browserproto.CDPTargetsResult{Targets: rows[start:end], Truncated: end < uint(len(rows))})
}

func debugSend(
	ctx context.Context,
	operation *Operation,
	connection *DebugConnection,
	debug *DebugOwner,
	caller uint64,
	input *browserproto.CDPSendInput,
) (json.RawMessage, error) {
	selected := "main"
	if input.Target != nil {
		selected = *input.Target
	}
	var result json.RawMessage
	err := operation.Run(ctx, func(work context.Context) error {
		var err error
		result, err = connection.Send(work, input.Method, json.RawMessage(input.Params), selected)
		return err
	})
	if ErrorCode(err) == "timeout" || ErrorCode(err) == "cancelled" {
		err = AfterCleanup(err, debug.Detach(context.WithoutCancel(ctx), caller))
	}
	return commandValue(browserproto.CDPSendResult{Method: input.Method, Result: result}, err)
}

func debugEvents(
	ctx context.Context,
	operation *Operation,
	connection *DebugConnection,
	debug *DebugOwner,
	caller uint64,
	input *browserproto.CDPEventsInput,
) (json.RawMessage, error) {
	query := EventQuery{After: input.After, Limit: browserproto.DefaultNodes, Target: "main"}
	if input.Method != nil {
		query.Methods = *input.Method
		for _, method := range query.Methods {
			if _, err := pinnedSchema(method, "event"); err != nil {
				return nil, err
			}
		}
	}
	if input.Target != nil {
		query.Target = *input.Target
	}
	if input.Limit != nil {
		query.Limit = *input.Limit
	}
	targets, err := connection.Targets(operation.Context())
	if err != nil {
		return nil, err
	}
	if !slices.Contains(targets, query.Target) {
		return nil, &BrowserError{Kind: KindTargetNotFound}
	}
	for {
		recorded := debug.Recorded()
		page, err := debug.Events(operation.Context(), query)
		if err != nil {
			return nil, err
		}
		if !page.Wait || input.TimeoutMS == nil {
			return Value(page.Result)
		}
		err = operation.Run(ctx, func(work context.Context) error {
			select {
			case <-recorded:
				return nil
			case <-work.Done():
				return work.Err()
			}
		})
		if ctx.Err() != nil {
			return nil, AfterCleanup(err, debug.Detach(context.WithoutCancel(ctx), caller))
		}
		if IsDeadline(err) {
			return Value(page.Result)
		}
		if err != nil {
			return nil, err
		}
	}
}

func (d *DebugOwner) existingConnection(caller uint64) (*DebugConnection, error) {
	d.mu.Lock()
	existing := d.connections[caller]
	if existing == nil && d.buffer == nil {
		buffer, err := NewBuffer(
			"cdp",
			browserproto.CDPEvents,
			browserproto.CDPBytes,
			func(e browserproto.CDPEvent) uint64 {
				return e.Sequence
			},
			func(e browserproto.CDPEvent, n uint64) browserproto.CDPEvent {
				e.Sequence = n
				return e
			},
		)
		if err != nil {
			d.mu.Unlock()
			return nil, err
		}
		d.buffer = buffer
	}
	d.mu.Unlock()

	return existing, nil
}
