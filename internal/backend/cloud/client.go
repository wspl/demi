package cloud

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"strconv"
	"sync/atomic"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
)

// Client owns the machine manager's socket, opened on first use and again by
// the next call after a drop. Calls are never replayed. The owner must Close it.
type Client struct {
	ctx      context.Context
	cancel   context.CancelFunc
	commands chan clientCommand
	done     chan struct{}
	closing  atomic.Bool
	// dial is the socket boundary; tests use a scripted in-memory connection.
	dial func(context.Context, string, string) (net.Conn, error)
}
type clientCommand struct {
	call       machinewire.Call
	disconnect bool
	final      bool
	answer     chan clientAnswer
}
type clientAnswer struct {
	data []byte
	err  error
}
type receivedLine struct {
	data []byte
	err  error
}
type managerConnection struct {
	socket   net.Conn
	incoming chan receivedLine
	cancel   context.CancelFunc
	done     chan struct{}
}

// NewClient creates a client and the sole receiver of sandbox death events.
// ctx belongs to the backend lifetime. The bounded death queue backpressures
// the reader. The backend drains it until Close closes the channel.
func NewClient(ctx context.Context, socket string) (*Client, <-chan webapi.DeviceID) {
	life, cancel := context.WithCancel(ctx)
	c := &Client{
		ctx:      life,
		cancel:   cancel,
		commands: make(chan clientCommand, 64),
		done:     make(chan struct{}),
		dial:     (&net.Dialer{}).DialContext,
	}
	deaths := make(chan webapi.DeviceID, 256)
	go c.run(life, socket, deaths)
	return c, deaths
}

// Call runs params and decodes the reply through its operation contract.
// Cancellation ends the wait, never the manager's operation or a retry of it.
func Call[T any](ctx context.Context, client *Client, params machinewire.Operation[T]) (T, error) {
	var zero T
	call := params.Call()
	answer, err := client.command(ctx, clientCommand{call: call})
	if err != nil {
		return zero, err
	}
	value, err := params.DecodeOutput(answer)
	if err != nil {
		return zero, fmt.Errorf("the machine manager answered %s with an unexpected result: %w", call.Name(), err)
	}
	return value, nil
}

// Disconnect reconciles a live connection, then disconnects even on failure.
// Nothing connects solely to disconnect; subsequent calls can reconnect.
func (c *Client) Disconnect(ctx context.Context) error {
	_, err := c.command(ctx, clientCommand{call: &machinewire.Reconcile{}, disconnect: true})
	return err
}

// Close permanently stops admission, reconciles a live socket and joins workers.
// A canceled cleanup context forces the socket closed and still joins workers.
func (c *Client) Close(ctx context.Context) error {
	var err error
	if c.closing.CompareAndSwap(false, true) {
		_, err = c.command(ctx, clientCommand{call: &machinewire.Reconcile{}, disconnect: true, final: true})
		c.cancel()
	}
	select {
	case <-c.done:
	case <-ctx.Done():
		c.cancel()
		<-c.done
		if err == nil {
			err = ctx.Err()
		}
	}
	return err
}

// command submits a manager operation without giving its caller socket ownership.
func (c *Client) command(ctx context.Context, command clientCommand) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if c.closing.Load() && !command.final {
		return nil, unavailable(command.call.Name(), errors.New("the machine manager's client is closed"))
	}
	command.answer = make(chan clientAnswer, 1)
	select {
	case c.commands <- command:
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-c.done:
		return nil, unavailable(command.call.Name(), errors.New("the machine manager's client is closed"))
	}
	select {
	case a := <-command.answer:
		return a.data, a.err
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-c.done:
		select {
		case a := <-command.answer:
			return a.data, a.err
		default:
			return nil, unavailable(command.call.Name(), errors.New("the machine manager's client is closed"))
		}
	}
}

// unavailable describes an uncertain manager outcome and keeps its cause.
func unavailable(operation string, err error) error {
	//nolint:staticcheck // Product text, shown to the user as it is.
	return fmt.Errorf("Machine manager unavailable during %s: %w", operation, err)
}

// run exclusively owns connection identity, request IDs and pending calls.
func (c *Client) run(ctx context.Context, socket string, deaths chan<- webapi.DeviceID) {
	defer close(c.done)
	defer close(deaths)
	var connection *managerConnection
	pending := make(map[string]clientCommand)
	var next uint64 = 1
	drop := func(reason error) {
		if connection != nil {
			_ = connection.close(context.WithoutCancel(ctx)) // Teardown joins without a cancellation deadline.
			connection = nil
		}
		for id, p := range pending {
			p.answer <- clientAnswer{err: unavailable(p.call.Name(), reason)}
			delete(pending, id)
		}
	}
	defer func() { drop(errors.New("the machine manager's client is closed")) }()
	for {
		var incoming <-chan receivedLine
		if connection != nil {
			incoming = connection.incoming
		}
		select {
		case <-ctx.Done():
			return
		case command := <-c.commands:
			if c.sendCommand(ctx, socket, command, &connection, pending, &next, drop) {
				return
			}
		case line := <-incoming:
			if receiveManagerLine(ctx, line, pending, deaths, drop) {
				return
			}
		}
	}
}

// readManager owns a reader and cancellation watcher for one manager connection.
func readManager(ctx context.Context, conn net.Conn) *managerConnection {
	life, cancel := context.WithCancel(ctx)
	c := &managerConnection{
		socket:   conn,
		incoming: make(chan receivedLine, 64),
		cancel:   cancel,
		done:     make(chan struct{}),
	}
	go func() {
		defer close(c.done)
		closed := make(chan struct{})
		stop := context.AfterFunc(life, func() {
			// Closing an already-failed connection cannot lose accepted work.
			_ = conn.Close()
			close(closed)
		})
		defer func() {
			// The reader is ending; only descriptor cleanup remains.
			_ = conn.Close()
			if !stop() {
				<-closed
			}
		}()
		scanner := bufio.NewScanner(conn)
		scanner.Buffer(make([]byte, 4096), machinewire.MaxLineBytes+2)
		for scanner.Scan() {
			data := append([]byte(nil), scanner.Bytes()...)
			if len(data) > machinewire.MaxLineBytes {
				break
			}
			select {
			case c.incoming <- receivedLine{data: data}:
			case <-life.Done():
				return
			}
		}
		err := scanner.Err()
		if err == nil {
			err = errors.New("the machine manager closed the connection")
		}
		// Closing also unblocks a supervisor whose write met a dead peer.
		_ = conn.Close()
		select {
		case c.incoming <- receivedLine{err: err}:
		case <-life.Done():
		}
	}()
	return c
}

// close releases the connection and joins its reader, including cancellation IO.
func (c *managerConnection) close(ctx context.Context) error {
	c.cancel()
	// The supervisor already failed or answered the pending calls.
	_ = c.socket.Close()
	select {
	case <-c.done:
		return nil
	case <-ctx.Done():
		<-c.done
		return ctx.Err()
	}
}

// sendCommand opens a connection as needed and registers an operation before writing it.
// Its result tells the supervisor when a final command has ended the client.
func (c *Client) sendCommand(
	ctx context.Context,
	socket string,
	command clientCommand,
	connection **managerConnection,
	pending map[string]clientCommand,
	next *uint64,
	drop func(error),
) bool {
	if c.closing.Load() && !command.final {
		command.answer <- clientAnswer{err: unavailable(command.call.Name(),
			errors.New("the machine manager's client is closed"))}
		return false
	}
	if *connection == nil {
		if command.disconnect {
			command.answer <- clientAnswer{}
			return command.final
		}
		conn, err := c.dial(ctx, "unix", socket)
		if err != nil {
			command.answer <- clientAnswer{err: unavailable(command.call.Name(), fmt.Errorf("%s: %w", socket, err))}
			return false
		}
		*connection = readManager(ctx, conn)
	}
	id := strconv.FormatUint(*next, 10)
	*next++
	line, err := machinewire.EncodeLine(machinewire.MachineRequest{ID: id, Call: command.call})
	if err != nil {
		command.answer <- clientAnswer{err: err}
		return false
	}
	pending[id] = command
	if _, err = (*connection).socket.Write(line); err != nil {
		drop(err)
		if command.final {
			return true
		}
	}
	return false
}

// receiveManagerLine delivers deaths or answers and reconciles disconnect commands.
// Its result tells the supervisor when a final reply or cancellation ends the client.
func receiveManagerLine(
	ctx context.Context,
	line receivedLine,
	pending map[string]clientCommand,
	deaths chan<- webapi.DeviceID,
	drop func(error),
) bool {
	if line.err != nil {
		final := false
		for _, p := range pending {
			final = final || p.final
		}
		drop(line.err)
		return final
	}
	if len(line.data) == 0 {
		return false
	}
	id, answer, reply, stop := managerAnswer(ctx, line.data, deaths)
	if !reply {
		return stop
	}
	p, ok := pending[id]
	if !ok {
		slog.Warn("the machine manager answered a request this backend did not send", "id", id)
		return false
	}
	delete(pending, id)
	if p.disconnect {
		if answer.err == nil {
			_, answer.err = machinewire.ReconcileParams{}.DecodeOutput(answer.data)
			if answer.err != nil {
				answer.err = fmt.Errorf(
					"the machine manager answered %s with an unexpected result: %w",
					p.call.Name(),
					answer.err,
				)
			}
		}
		drop(errors.New("the backend disconnected"))
	}
	p.answer <- answer
	return p.final
}

// managerAnswer decodes one response and forwards death events before any reply delivery.
func managerAnswer(ctx context.Context, data []byte, deaths chan<- webapi.DeviceID) (string, clientAnswer, bool, bool) {
	response, err := machinewire.DecodeResponse(data)
	if err != nil {
		slog.Warn("an unreadable line from the machine manager was dropped", "error", err)
		return "", clientAnswer{}, false, false
	}
	var id string
	var answer clientAnswer
	switch response := response.(type) {
	case *machinewire.Death:
		device, err := webapi.ParseDeviceID(response.DeviceID)
		if err != nil {
			slog.Warn("the machine manager reported an invalid device death", "error", err)
			return "", clientAnswer{}, false, false
		}
		select {
		case deaths <- device:
		case <-ctx.Done():
			return "", clientAnswer{}, false, true
		}
		return "", clientAnswer{}, false, false
	case *machinewire.OK:
		id = response.ID
		answer.data = response.Result
	case *machinewire.ErrorResponse:
		id = response.ID
		answer.err = errors.New(response.Message)
	}
	return id, answer, true, false
}
