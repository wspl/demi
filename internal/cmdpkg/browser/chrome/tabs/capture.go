package tabs

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/contract"

	"github.com/chromedp/cdproto/target"
)

// Frame is one encoded picture received from the capture extension.
// Data is immutable after publication.
type Frame struct {
	// Sequence identifies the frame within the capture.
	Sequence uint32
	// Key reports whether the frame can begin decoding.
	Key bool
	// Timestamp holds the encoded presentation timestamp.
	Timestamp float64
	// Width holds the encoded width in pixels.
	Width uint16
	// Height holds the encoded height in pixels.
	Height uint16
	// Data retains the encoded video bytes.
	Data []byte
}

// CaptureEvent is a validated event from the extension connection.
//
//sumtype:decl
type CaptureEvent interface{ captureEvent() }

// CaptureStarted reports that the extension started the requested capture.
type CaptureStarted struct{}

func (*CaptureStarted) captureEvent() {}

// CaptureFrame carries one encoded picture.
type CaptureFrame struct {
	// Frame holds the encoded frame.
	Frame Frame
}

func (*CaptureFrame) captureEvent() {}

// CaptureStalled reports that the page stopped painting before capture began.
type CaptureStalled struct{}

func (*CaptureStalled) captureEvent() {}

// CaptureFailed carries the extension's capture failure reason.
type CaptureFailed struct {
	// Reason describes the capture failure.
	Reason string
}

func (*CaptureFailed) captureEvent() {}

// CaptureChannel reaches the environment-owned capture listener and connection.
// Replacement connections retire old captures before accepting replacement work.
type CaptureChannel struct {
	ctx      context.Context
	requests chan captureRequest
	sockets  chan *websocket.Conn
	inbound  chan captureInbound
	done     chan struct{}
	// mu protects connection publication only; it is never held across IO.
	mu        sync.Mutex
	connected bool
	changed   chan struct{}
}

// Start captures a target at the requested encoded dimensions. The extension's
// initial connection has Rust's ten-second bound; failure does not close the tab.
func (c *CaptureChannel) Start(
	ctx context.Context,
	targetID target.ID,
	width, height, fps, bitrate uint32,
) (*Capture, error) {
	if reason := captureUnavailable(); reason != "" {
		return nil, &cdp.BrowserError{Kind: cdp.KindUnsupportedCapability, Message: reason}
	}
	connected, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	for {
		c.mu.Lock()
		ready, changed := c.connected, c.changed
		c.mu.Unlock()
		if ready {
			break
		}
		select {
		case <-c.ctx.Done():
			return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-connected.Done():
			return nil, &cdp.BrowserError{Kind: cdp.KindUnavailable, Message: "the capture extension did not connect"}
		case <-changed:
		}
	}
	reply := make(chan captureReply, 1)
	request := captureRequest{
		start: &browserop.CaptureCommandStart{
			Target:  string(targetID),
			Width:   width,
			Height:  height,
			FPS:     fps,
			Bitrate: bitrate,
		},
		reply: reply,
	}
	select {
	case c.requests <- request:
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-c.ctx.Done():
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	// An admitted start always answers; if its caller left, close the capture
	// before reporting cancellation so it cannot outlive its consumer.
	select {
	case answer := <-reply:
		if err := ctx.Err(); err != nil {
			if answer.capture != nil {
				cleanup, stop := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
				defer stop()
				return nil, cdp.AfterCleanup(err, answer.capture.Close(cleanup))
			}
			return nil, err
		}
		return answer.capture, answer.err
	case <-c.done:
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
}

// Capture owns one capture subscription; its consumer must await Close.
type Capture struct {
	channel   *CaptureChannel
	id        uint32
	events    chan CaptureEvent
	closeOnce sync.Once
	closed    chan struct{}
}

// Next waits for the next event or connection end. Frame payloads are immutable.
func (c *Capture) Next(ctx context.Context) (CaptureEvent, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case event, ok := <-c.events:
		if !ok {
			return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
		}
		return event, nil
	}
}

// Ack reports received frames and the maximum in-flight frame window.
// Like Rust, superseding controls are dropped when the control queue is full.
func (c *Capture) Ack(sequence, window uint32) {
	c.command(&browserop.CaptureCommandAck{Capture: c.id, Sequence: sequence, Window: window})
}

// KeyFrame requests a key frame through the bounded, nonblocking control queue.
func (c *Capture) KeyFrame() {
	c.command(&browserop.CaptureCommandKeyframe{Capture: c.id})
}

// Encoding changes bitrate and frame rate through the superseding control queue.
func (c *Capture) Encoding(bitrate, fps uint32) {
	c.command(&browserop.CaptureCommandEncoding{Capture: c.id, Bitrate: bitrate, FPS: fps})
}

// Close stops capture and releases its event queue, including on cancellation.
func (c *Capture) Close(ctx context.Context) error {
	c.closeOnce.Do(func() { close(c.closed) })
	reply := make(chan captureReply, 1)
	select {
	case c.channel.requests <- captureRequest{stop: c, reply: reply}:
	case <-c.channel.done:
		return ctx.Err()
	}
	select {
	case <-reply:
	case <-c.channel.done:
	}
	return ctx.Err()
}

type captureReply struct {
	capture *Capture
	err     error
}
type captureRequest struct {
	start   *browserop.CaptureCommandStart
	command browserop.CaptureCommand
	stop    *Capture
	reply   chan captureReply
}
type captureInbound struct {
	generation uint64
	id         uint32
	event      CaptureEvent
	ended      bool
}
type captureConnection struct {
	socket   *websocket.Conn
	commands chan browserop.CaptureCommand
	cancel   context.CancelFunc
	done     chan struct{}
}

// command queues superseding capture controls without stalling their consumer.
func (c *Capture) command(command browserop.CaptureCommand) {
	select {
	case <-c.closed:
		return
	default:
	}
	select {
	case c.channel.requests <- captureRequest{command: command}:
	default:
	}
}

// bindCapture binds the token-authenticated extension endpoint before Chrome starts.
func bindCapture(ctx context.Context) (*CaptureChannel, string, func() error, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, "", nil, err
	}
	token := make([]byte, 24)
	if _, err := rand.Read(token); err != nil {
		return nil, "", nil, errors.Join(err, listener.Close())
	}
	expected := "token=" + hex.EncodeToString(token)
	lifetime, cancel := context.WithCancel(ctx)
	c := &CaptureChannel{
		ctx:      lifetime,
		requests: make(chan captureRequest, 64),
		sockets:  make(chan *websocket.Conn),
		inbound:  make(chan captureInbound, 16),
		done:     make(chan struct{}),
		changed:  make(chan struct{}),
	}
	server := &http.Server{BaseContext: func(net.Listener) context.Context { return lifetime }}
	var handlers sync.WaitGroup
	var admission sync.Mutex
	closing := false
	server.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		admission.Lock()
		if closing {
			admission.Unlock()
			return
		}
		handlers.Add(1)
		admission.Unlock()
		defer handlers.Done()
		c.accept(lifetime, w, r, expected)
	})
	served := make(chan struct{})
	go func() {
		defer close(served)
		_ = server.Serve(listener)
	}() // Close owns the listener; its final serve error has no caller work.
	go c.run()
	closeAll := func() error {
		admission.Lock()
		closing = true
		admission.Unlock()
		cancel()
		err := server.Close()
		<-served
		handlers.Wait()
		<-c.done
		return err
	}
	return c, "ws://" + listener.Addr().String() + "/?" + expected, closeAll, nil
}

// publishConnection wakes starts from the same immutable connection publication.
func (c *CaptureChannel) publishConnection(connected bool) {
	c.mu.Lock()
	old := c.changed
	c.connected = connected
	c.changed = make(chan struct{})
	c.mu.Unlock()
	close(old)
}

// run owns the extension generation and the routes on that connection.
func (c *CaptureChannel) run() {
	defer close(c.done)
	routes := make(map[uint32]*Capture)
	var connection *captureConnection
	var connectionDone <-chan struct{}
	var generation uint64
	var next uint32
	disconnect := func() {
		if connection != nil {
			connection.cancel()
			_ = connection.socket.CloseNow() // A disconnected socket has no remaining protocol obligation.
			<-connection.done
			connection = nil
			connectionDone = nil
		}
		c.publishConnection(false)
		for id, capture := range routes {
			select {
			case capture.events <- &CaptureFailed{Reason: "capture extension disconnected"}:
			default:
			}
			close(capture.events)
			delete(routes, id)
		}
	}
	defer disconnect()
	for {
		select {
		case <-c.ctx.Done():
			return
		case socket := <-c.sockets:
			disconnect()
			generation++
			connection = c.connect(socket, generation)
			connectionDone = connection.done
			c.publishConnection(true)
		case <-connectionDone:
			disconnect()
		case message := <-c.inbound:
			if message.generation != generation {
				continue
			}
			if message.ended {
				disconnect()
				continue
			}
			if capture := routes[message.id]; capture != nil && message.event != nil {
				select {
				case capture.events <- message.event:
				default:
				} // Drop newest pictures for a lagging consumer.
			}
		case request := <-c.requests:
			c.requestCapture(request, connection, routes, &next, disconnect)
		}
	}
}

// connect owns and joins both directions of one capture extension socket.
func (c *CaptureChannel) connect(socket *websocket.Conn, generation uint64) *captureConnection {
	ctx, cancel := context.WithCancel(c.ctx)
	connection := &captureConnection{
		socket:   socket,
		commands: make(chan browserop.CaptureCommand, 64),
		cancel:   cancel,
		done:     make(chan struct{}),
	}
	go func() {
		defer close(connection.done)
		written := make(chan struct{})
		go func() {
			writeCapture(ctx, socket, connection, written, cancel)
		}()
		for {
			kind, data, err := socket.Read(ctx)
			if err != nil {
				break
			}
			message, err := decodeCapture(kind, data)
			if err != nil {
				slog.Warn("capture extension message refused", "error", err)
				break
			}
			message.generation = generation
			select {
			case c.inbound <- message:
			case <-ctx.Done():
			}
			if ctx.Err() != nil {
				break
			}
		}
		cancel()
		_ = socket.CloseNow() // Termination is established by joining both workers.
		<-written
		select {
		case c.inbound <- captureInbound{generation: generation, ended: true}:
		case <-ctx.Done():
		}
	}()
	return connection
}

// decodeCapture validates each extension message before routing any capture data.
func decodeCapture(kind websocket.MessageType, data []byte) (captureInbound, error) {
	if kind == websocket.MessageBinary {
		header, payload, err := browserop.SplitCaptureFrame(data)
		if err != nil {
			return captureInbound{}, err
		}
		return captureInbound{
			id: header.Capture,
			event: &CaptureFrame{
				Frame: Frame{
					Sequence:  header.Sequence,
					Key:       header.Key,
					Timestamp: header.Timestamp,
					Width:     header.Width,
					Height:    header.Height,
					Data:      payload,
				},
			},
		}, nil
	}
	event, err := browserop.DecodeCaptureEvent(data)
	if err != nil {
		return captureInbound{}, err
	}
	switch event := event.(type) {
	case *browserop.CaptureEventReady, *browserop.CaptureEventStopped:
		return captureInbound{}, nil
	case *browserop.CaptureEventStarted:
		return captureInbound{id: event.Capture, event: &CaptureStarted{}}, nil
	case *browserop.CaptureEventStalled:
		return captureInbound{id: event.Capture, event: &CaptureStalled{}}, nil
	case *browserop.CaptureEventError:
		return captureInbound{id: event.Capture, event: &CaptureFailed{Reason: event.Message}}, nil
	}
	return captureInbound{}, nil
}

func (c *CaptureChannel) accept(ctx context.Context, w http.ResponseWriter, r *http.Request, expected string) {
	if r.URL.RawQuery != expected {
		http.Error(w, "", http.StatusForbidden)
		return
	}
	socket, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return
	}
	socket.SetReadLimit(64 * 1024 * 1024) // tungstenite's default message limit.
	select {
	case c.sockets <- socket:
	case <-ctx.Done():
		_ = socket.CloseNow() // The environment already ended.
	}
}

func (c *CaptureChannel) requestCapture(
	request captureRequest,
	connection *captureConnection,
	routes map[uint32]*Capture,
	next *uint32,
	disconnect func(),
) {
	switch {
	case request.start != nil:
		if connection == nil {
			request.reply <- captureReply{
				err: &cdp.BrowserError{
					Kind:    cdp.KindUnavailable,
					Message: "the capture extension is not connected",
				},
			}
			return
		}
		id := *next
		*next++
		request.start.Capture = id
		select {
		case connection.commands <- request.start:
			capture := &Capture{
				channel: c,
				id:      id,
				events:  make(chan CaptureEvent, 8),
				closed:  make(chan struct{}),
			}
			routes[id] = capture
			request.reply <- captureReply{capture: capture}
		default:
			disconnect()
			request.reply <- captureReply{
				err: &cdp.BrowserError{
					Kind:    cdp.KindUnavailable,
					Message: "the capture extension is not keeping up",
				},
			}
		}
	case request.stop != nil:
		if routes[request.stop.id] != request.stop {
			request.reply <- captureReply{}
			return
		}
		delete(routes, request.stop.id)
		close(request.stop.events)
		stopCapture(connection, request, disconnect)
		request.reply <- captureReply{}
	case request.command != nil:
		if connection != nil {
			select {
			case connection.commands <- request.command:
			default:
			}
		}
	}
}

func writeCapture(
	ctx context.Context,
	socket *websocket.Conn,
	connection *captureConnection,
	written chan struct{},
	cancel context.CancelFunc,
) {
	defer close(written)
	defer cancel()
	for {
		select {
		case <-ctx.Done():
			return
		case command := <-connection.commands:
			data, err := contract.EncodeJSON(command)
			if err != nil {
				slog.Warn("capture command refused", "error", err)
				return
			}
			if err := socket.Write(ctx, websocket.MessageText, data); err != nil {
				return
			}
		}
	}
}

func stopCapture(connection *captureConnection, request captureRequest, disconnect func()) {
	if connection != nil {
		select {
		case connection.commands <- &browserop.CaptureCommandStop{Capture: request.stop.id}:
		default:
			disconnect()
		}
	}
}
