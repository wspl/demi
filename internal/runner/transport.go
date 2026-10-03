package runner

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerwire"
)

// socketURL selects the backend's runner route without losing its query.
func socketURL(backend runnerwire.BackendURL) (string, error) {
	u, err := url.Parse(backend.String())
	if err != nil {
		return "", err
	}
	switch u.Scheme {
	case "http", "ws":
		u.Scheme = "ws"
	default:
		u.Scheme = "wss"
	}
	if u.Path == "" || u.Path == "/" {
		u.Path = "/api/runner"
	}
	return u.String(), nil
}

// transport owns the backend socket and both bounded directions of traffic.
// Its caller joins all senders before close flushes their queued frames.
type transport struct {
	output   chan []byte
	control  chan []byte
	input    chan runnerwire.Inbound
	socket   *websocket.Conn
	cancel   context.CancelFunc
	done     chan struct{}
	stopping chan struct{}
	once     sync.Once
	err      error
}

func connect(ctx context.Context, backend runnerwire.BackendURL) (*transport, error) {
	address, err := socketURL(backend)
	if err != nil {
		return nil, err
	}
	opening, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	socket, response, err := websocket.Dial(opening, address, nil)
	if err != nil {
		if response != nil && response.Body != nil {
			_ = response.Body.Close()
		} // Dial already closes failed handshakes; ensure no retained body.
		return nil, fmt.Errorf("backend connection: %w", err)
	}
	return newTransport(socket), nil
}

func newTransport(socket *websocket.Conn) *transport {
	ctx, cancel := context.WithCancel(context.Background())
	t := &transport{
		output:   make(chan []byte, 8),
		control:  make(chan []byte, 128),
		input:    make(chan runnerwire.Inbound, 8),
		socket:   socket,
		cancel:   cancel,
		done:     make(chan struct{}),
		stopping: make(chan struct{}),
	}
	socket.SetReadLimit(runnerwire.MaxMessageBytes)
	go t.run(ctx)
	return t
}

func (t *transport) run(ctx context.Context) {
	defer close(t.done)
	defer close(t.input)
	defer t.cancel()
	defer func() { _ = t.socket.CloseNow() }() // The result is decided by the read/write or graceful shutdown.
	received := make(chan error, 1)
	go func() { received <- t.receive(ctx) }()
	sent := make(chan error, 1)
	go func() { sent <- t.send(ctx) }()
	writerJoined := false
	select {
	case err := <-received:
		t.err = err
		t.cancel()
		<-sent
		return
	case err := <-sent:
		if err != nil {
			t.err = err
			t.cancel()
			<-received
			return
		}
		writerJoined = true
	case <-t.stopping:
	}
	// The close deadline includes finishing the current write and flushing.
	expired := make(chan struct{})
	deadline := time.AfterFunc(5*time.Second, func() {
		defer close(expired)
		t.cancel()
		_ = t.socket.CloseNow() // Shutdown may already have closed it.
	})
	if !writerJoined {
		<-sent
	}
	// Like Rust, an unsuccessful orderly close cannot change the work result.
	_ = t.flush(ctx)
	_ = t.socket.Close(websocket.StatusGoingAway, "runner stopping")
	t.cancel()
	<-received
	if !deadline.Stop() {
		<-expired
	}
}

func (t *transport) receive(ctx context.Context) error {
	for {
		kind, data, err := t.socket.Read(ctx)
		if err != nil {
			if websocket.CloseStatus(err) != -1 {
				return nil
			}
			return err
		}
		if kind != websocket.MessageBinary {
			return errors.New("runner requires binary WebSocket messages")
		}
		message, err := runnerwire.DecodeInbound(data)
		if err != nil {
			return err
		}
		select {
		case t.input <- message:
		case <-ctx.Done():
			return ctx.Err()
		case <-t.stopping:
			// Continue reading control frames while shutdown flushes output.
		}
	}
}

func (t *transport) send(ctx context.Context) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-t.stopping:
			return nil
		default:
		}
		var data []byte
		select {
		case data = <-t.control:
		default:
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-t.stopping:
				return nil
			case data = <-t.control:
			case data = <-t.output:
			}
		}
		if err := t.write(ctx, data); err != nil {
			return err
		}
	}
}

func (t *transport) write(ctx context.Context, data []byte) error {
	if len(data) > runnerwire.MaxMessageBytes {
		return fmt.Errorf("runner outbound message exceeds %d bytes", runnerwire.MaxMessageBytes)
	}
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	return t.socket.Write(ctx, websocket.MessageBinary, data)
}

func (t *transport) flush(ctx context.Context) error {
	for {
		var data []byte
		select {
		case data = <-t.control:
		default:
			select {
			case data = <-t.output:
			default:
				return nil
			}
		}
		if err := t.write(ctx, data); err != nil {
			return err
		}
	}
}

func (t *transport) close(ctx context.Context) error {
	t.once.Do(func() { close(t.stopping) })
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-t.done:
		return t.err
	}
}
