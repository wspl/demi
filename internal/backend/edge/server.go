package edge

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"sync"
	"sync/atomic"
)

type connectionKey struct{}

// Edge is the listener and the server over it. Obtain it with Start and share
// its pointer; its owner must call Close before disposing shared services.
type Edge struct {
	listener    net.Listener
	address     netip.AddrPort
	state       AppState
	handler     http.Handler
	closing     atomic.Bool
	cancel      context.CancelFunc
	done        chan struct{}
	mu          sync.Mutex
	connections map[net.Conn]struct{}
	workers     sync.WaitGroup
}

// Start binds address and starts serving the API and, when webDirectory is
// nonempty, the built web app in that directory. Port zero selects a free port.
// It publishes the listening address to state.Services.PublicURL before serving
// the first request. A failed start releases resources acquired by that attempt.
// ctx owns the edge's lifetime; cancellation closes its connections and listener.
// The caller must still call Close to join its work.
func Start(ctx context.Context, address netip.AddrPort, state AppState, webDirectory string) (*Edge, error) {
	// "tcp" would make an unspecified IPv4 address a dual-stack socket that
	// also accepts IPv6; the backend listens on exactly the address's family.
	network := "tcp6"
	if address.Addr().Unmap().Is4() {
		network = "tcp4"
	}
	listener, err := (&net.ListenConfig{}).Listen(ctx, network, address.String())
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancel(ctx)
	e := &Edge{listener: listener, address: listener.Addr().(*net.TCPAddr).AddrPort(), state: state, cancel: cancel, done: make(chan struct{}), connections: make(map[net.Conn]struct{})}
	state.Services.PublicURL.Listening(state.Site.PublicURL, e.address)
	e.handler = e.routes(webDirectory)
	go e.serve(lifetime)
	return e, nil
}

// LocalAddr returns the bound listener address, including the selected port.
func (e *Edge) LocalAddr() netip.AddrPort {
	return e.address
}

// StopAccepting closes the listener. Open connections go on serving, but a new
// request on one answers 503 backend_closing. Runner pipes remain available
// because backend shutdown needs them. This method is idempotent.
func (e *Edge) StopAccepting() {
	e.closing.Store(true)
	// Closing an already closed listener is harmless; accept observes the result.
	_ = e.listener.Close()
}

// Close closes the listener and connections still open, such as a download's,
// and waits for the server and its request and copy workers to end. It is
// idempotent. If ctx ends before the join finishes, shutdown continues; the owner
// must call Close again with a live context to finish joining before disposing
// shared services. Shard-owned sockets are drained by the shards before Close.
func (e *Edge) Close(ctx context.Context) error {
	e.StopAccepting()
	e.cancel()
	select {
	case <-e.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (e *Edge) serve(ctx context.Context) {
	defer close(e.done)
	stopped := make(chan struct{})
	go func() {
		defer close(stopped)
		<-ctx.Done()
		e.StopAccepting()
		e.mu.Lock()
		sockets := make([]net.Conn, 0, len(e.connections))
		for conn := range e.connections {
			sockets = append(sockets, conn)
		}
		e.mu.Unlock()
		for _, conn := range sockets {
			_ = conn.Close()
		}
	}()
	for {
		conn, err := e.listener.Accept()
		if err != nil {
			break
		}
		e.mu.Lock()
		if ctx.Err() != nil {
			e.mu.Unlock()
			_ = conn.Close()
			break
		}
		connectionCtx, incoming := readIncoming(ctx, conn)
		conn = newActivity(incoming)
		e.connections[conn] = struct{}{}
		e.mu.Unlock()
		e.workers.Add(1)
		go func() {
			defer e.workers.Done()
			defer func() {
				_ = conn.Close()
				e.mu.Lock()
				delete(e.connections, conn)
				e.mu.Unlock()
			}()
			e.serveConnection(connectionCtx, conn)
		}()
	}
	<-stopped
	e.workers.Wait()
}

func (e *Edge) serveConnection(ctx context.Context, conn net.Conn) {
	input := bufio.NewReader(conn)
	for {
		head, err := readHead(input)
		if err != nil {
			return
		}
		// Expose hostnames are selected from the original head, before net/http
		// parses or canonicalizes a header. This repeats for every keep-alive request.
		if label, ok := e.exposeLabel(head); ok {
			next, reusable := e.relay(ctx, conn, input, head, label)
			if !reusable {
				return
			}
			input = next
			continue
		}
		input = bufio.NewReader(io.MultiReader(bytes.NewReader(head), input))
		request, err := http.ReadRequest(input)
		if err != nil {
			return
		}
		body := trackBody(request, conn)
		request.RemoteAddr = conn.RemoteAddr().String()
		request = request.WithContext(context.WithValue(ctx, connectionKey{}, conn))
		writer := &response{conn: conn, input: input, request: request, header: make(http.Header)}
		func() {
			defer func() {
				if failure := recover(); failure != nil {
					slog.Error("a request failed", "panic", failure)
					if writer.status == 0 && !writer.hijacked {
						writeError(writer, internalError(errors.New("a request failed")))
					}
				}
			}()
			e.handler.ServeHTTP(writer, request)
		}()
		if writer.hijacked {
			return
		}
		err = writer.finish()
		// A unread body may block draining, so close the connection rather than
		// parsing its bytes as another request when a response has failed.
		if err != nil {
			return
		}
		if !body.complete {
			// A handler may refuse before reading a declared body. Closing first
			// prevents Body.Close from waiting for an upload the caller never sends.
			_ = conn.Close()
		}
		if err := request.Body.Close(); err != nil {
			return
		}
		if request.Close || !body.complete {
			return
		}
	}
}

func readHead(input *bufio.Reader) ([]byte, error) {
	var head []byte
	for {
		line, err := input.ReadBytes('\n')
		if err != nil {
			return nil, err
		}
		head = append(head, line...)
		if bytes.Equal(line, []byte("\r\n")) || bytes.Equal(line, []byte("\n")) {
			return head, nil
		}
	}
}
