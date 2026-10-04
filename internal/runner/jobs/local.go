package jobs

import (
	"context"
	"errors"
	"net"
	"sync"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

// Server owns the private local endpoint and every command client it accepts.
// Its owner must Close it, including after the lifetime context is cancelled.
type Server struct {
	listener *Listener
	lifetime context.Context
	cancel   context.CancelFunc
	done     chan struct{}
	// mu protects active count and notification, never connection IO.
	mu      sync.Mutex
	active  int
	changed chan struct{}
	err     error
}

// StartServer binds the endpoint and serves handler until ctx ends or Close runs.
func StartServer(ctx context.Context, handler cmdsdk.Handler[cmdproto.LocalInvocation]) (*Server, error) {
	listener, err := BindListener(ctx)
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancel(ctx)
	server := &Server{
		listener: listener,
		lifetime: lifetime,
		cancel:   cancel,
		done:     make(chan struct{}),
		changed:  make(chan struct{}),
	}
	go server.serve(handler)
	return server, nil
}

// Endpoint returns the command clients' endpoint.
func (s *Server) Endpoint() string {
	return s.listener.Endpoint()
}

// WaitIdle waits until no local clients remain connected.
func (s *Server) WaitIdle(ctx context.Context) error {
	for {
		s.mu.Lock()
		active, changed := s.active, s.changed
		s.mu.Unlock()
		if active == 0 {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}

// Close stops accepting, cancels and joins clients, and removes the endpoint.
// If ctx ends first, shutdown continues and a later Close can join it.
func (s *Server) Close(ctx context.Context) error {
	s.cancel()
	select {
	case <-s.done:
		return s.err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Listener owns a private socket and its liveness lock on Unix, or the local
// endpoint on Windows. Its owner must Close it; accepted connections belong to callers.
type Listener struct {
	endpoint string
	accept   func(context.Context) (net.Conn, error)
	close    func() error
	once     sync.Once
	err      error
}

// BindListener creates a local endpoint and its platform-specific liveness resources.
func BindListener(ctx context.Context) (*Listener, error) {
	return bindListener(ctx)
}

// Endpoint returns the local address for forwarding clients.
func (l *Listener) Endpoint() string {
	return l.endpoint
}

// Accept waits for a command connection. Cancelling ctx interrupts the wait.
func (l *Listener) Accept(ctx context.Context) (net.Conn, error) {
	return l.accept(ctx)
}

// Close releases the endpoint and liveness resources and wakes Accept.
func (l *Listener) Close() error {
	l.once.Do(func() {
		l.err = l.close()
	})
	return l.err
}

// serve owns the endpoint's accepted command connections and joins them on exit.
func (s *Server) serve(handler cmdsdk.Handler[cmdproto.LocalInvocation]) {
	defer close(s.done)
	var clients sync.WaitGroup
	var failureMu sync.Mutex
	var fatal error
	var backoff cmdsdk.Backoff
	for {
		conn, err := s.listener.Accept(s.lifetime)
		if err != nil {
			if s.lifetime.Err() != nil {
				break
			}
			if cmdsdk.Exhausted(err) {
				if backoff.Wait(s.lifetime) == nil {
					continue
				}
				break
			}
			s.err = err
			break
		}
		backoff = cmdsdk.Backoff{}
		s.changeActive(1)
		clients.Add(1)
		go func() {
			defer clients.Done()
			defer s.changeActive(-1)
			interrupted := make(chan struct{})
			stop := context.AfterFunc(s.lifetime, func() {
				_ = conn.Close()
				close(interrupted)
			})
			err := cmdsdk.ServeLocal(s.lifetime, conn, handler)
			if !stop() {
				<-interrupted
			}
			if errors.Is(err, cmdsdk.ErrConversationCleanup) || errors.Is(err, cmdsdk.ErrCancellationDeadline) {
				failureMu.Lock()
				fatal = errors.Join(fatal, err)
				failureMu.Unlock()
				s.cancel()
			}
		}()
	}
	s.cancel()
	s.err = errors.Join(s.err, s.listener.Close())
	clients.Wait()
	s.err = errors.Join(s.err, fatal)
}

// changeActive publishes the local endpoint's client count for drain waits.
func (s *Server) changeActive(delta int) {
	s.mu.Lock()
	s.active += delta
	old := s.changed
	s.changed = make(chan struct{})
	s.mu.Unlock()
	close(old)
}
