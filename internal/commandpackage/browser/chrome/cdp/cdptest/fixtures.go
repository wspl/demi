// Package cdptest supplies Chrome fixtures and scripted executors to browser tests.
package cdptest

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/chromedp/cdproto"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/cdproto/target"
	"github.com/coder/websocket"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

// EvaluateIn evaluates an expression over its own connection in a target that
// no public command addresses, such as the capture extension's worker.
// It closes its connection before returning, including on failure.
func EvaluateIn(
	ctx context.Context,
	address string,
	id target.ID,
	expression string,
) (value json.RawMessage, err error) {
	connection, err := cdp.Dial(ctx, address)
	if err != nil {
		return nil, err
	}
	defer func() {
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		defer cancel()
		err = cdp.AfterCleanup(err, connection.Close(cleanup))
	}()
	session, err := connection.Attach(ctx, id)
	if err != nil {
		return nil, err
	}
	result, exception, err := runtime.Evaluate(expression).
		WithReturnByValue(true).
		Do(protocol.WithExecutor(ctx, session))
	if err != nil {
		return nil, err
	}
	if exception != nil {
		return nil, fmt.Errorf("evaluation: %s", exception.Text)
	}
	if result == nil || result.Value == nil {
		return nil, fmt.Errorf("evaluation has no JSON value")
	}
	return json.RawMessage(result.Value), nil
}

// Exchange is one expected command and its scripted reply or failure.
// Params and Result use the same typed cdproto values as production callers.
type Exchange struct {
	// Method names the expected CDP command.
	Method string
	// SessionID identifies the expected flattened session.
	SessionID target.SessionID
	// Params holds the expected command parameters.
	Params any
	// Result supplies the scripted Chrome reply.
	Result any
	// Err supplies the scripted command failure.
	Err error
}

// Executor is a scripted cdproto executor. Test cleanup checks all exchanges
// were consumed; unexpected commands fail the test at the call boundary.
type Executor struct {
	t         testing.TB
	mu        sync.Mutex
	exchanges []Exchange
}

// NewExecutor installs the script and its test-owned cleanup verification.
func NewExecutor(t testing.TB, exchanges ...Exchange) *Executor {
	t.Helper()
	e := &Executor{t: t, exchanges: append([]Exchange(nil), exchanges...)}
	t.Cleanup(func() {
		e.mu.Lock()
		defer e.mu.Unlock()
		if len(e.exchanges) != 0 {
			t.Errorf("%d CDP exchanges not consumed, next %s", len(e.exchanges), e.exchanges[0].Method)
		}
	})
	return e
}

// take checks the next scripted Chrome request at its observable boundary.
func (e *Executor) take(method string, session target.SessionID, params any) (Exchange, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if len(e.exchanges) == 0 {
		err := fmt.Errorf("unexpected CDP command %s", method)
		e.t.Error(err)
		return Exchange{}, err
	}
	exchange := e.exchanges[0]
	e.exchanges = e.exchanges[1:]
	if exchange.Method != method || exchange.SessionID != session {
		err := fmt.Errorf("CDP command %s/%s, want %s/%s", session, method, exchange.SessionID, exchange.Method)
		e.t.Error(err)
		return Exchange{}, err
	}
	actual, err := jsonv2.Marshal(params)
	if err != nil {
		return Exchange{}, err
	}
	expected, err := jsonv2.Marshal(exchange.Params)
	if err != nil {
		return Exchange{}, err
	}
	var got, want any
	if err := jsonv2.Unmarshal(actual, &got); err != nil {
		return Exchange{}, err
	}
	if err := jsonv2.Unmarshal(expected, &want); err != nil {
		return Exchange{}, err
	}
	if !reflect.DeepEqual(got, want) {
		err := fmt.Errorf("CDP %s params %s, want %s", method, actual, expected)
		e.t.Error(err)
		return Exchange{}, err
	}
	return exchange, nil
}

// Execute checks a command against the next exchange and supplies its reply.
func (e *Executor) Execute(ctx context.Context, method string, params, result any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	exchange, err := e.take(method, "", params)
	if err != nil {
		return err
	}
	if exchange.Err != nil {
		return exchange.Err
	}
	if result == nil {
		return nil
	}
	data, err := jsonv2.Marshal(exchange.Result)
	if err != nil {
		return err
	}
	return jsonv2.Unmarshal(data, result)
}

// Server is a local scripted Chrome WebSocket fixture, with no Chrome process.
// Test cleanup closes sockets and joins all workers.
type Server struct {
	t         testing.TB
	script    *Executor
	server    *httptest.Server
	ctx       context.Context
	cancel    context.CancelFunc
	mu        sync.Mutex
	sockets   map[*websocket.Conn]struct{}
	connected chan struct{}
	wg        sync.WaitGroup
	done      chan struct{}
	closeOnce sync.Once
	closing   bool
}

// NewServer starts a test-owned server on an ephemeral port.
func NewServer(t testing.TB, exchanges ...Exchange) *Server {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	s := &Server{
		t:         t,
		script:    NewExecutor(t, exchanges...),
		ctx:       ctx,
		cancel:    cancel,
		sockets:   map[*websocket.Conn]struct{}{},
		connected: make(chan struct{}, 1),
		done:      make(chan struct{}),
	}
	s.server = httptest.NewServer(http.HandlerFunc(s.serve))
	t.Cleanup(func() {
		if err := s.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return s
}

// serve owns one scripted Chrome socket until its peer or fixture closes.
func (s *Server) serve(w http.ResponseWriter, r *http.Request) {
	socket, err := websocket.Accept(w, r, nil)
	if err != nil {
		s.t.Error(err)
		return
	}
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		_ = socket.CloseNow()
		return
	}
	s.wg.Add(1)
	s.sockets[socket] = struct{}{}
	s.mu.Unlock()
	defer s.wg.Done()
	defer func() {
		_ = socket.CloseNow()
		s.mu.Lock()
		delete(s.sockets, socket)
		s.mu.Unlock()
	}()
	socket.SetReadLimit(cdp.MessageLimit)
	select {
	case s.connected <- struct{}{}:
	default:
	}
	for {
		_, data, err := socket.Read(s.ctx)
		if err != nil {
			return
		}
		var request cdproto.Message
		if err := jsonv2.Unmarshal(data, &request); err != nil {
			s.t.Error(err)
			return
		}
		if !s.respond(socket, request) {
			return
		}
	}
}

// Address returns the fixture's browser WebSocket address.
func (s *Server) Address() string {
	return "ws" + strings.TrimPrefix(s.server.URL, "http")
}

// Emit sends an event after the test's preceding command or synchronization.
func (s *Server) Emit(ctx context.Context, event cdp.Event) error {
	data, err := jsonv2.Marshal(struct {
		Method    string           `json:"method"`
		Params    jsontext.Value   `json:"params"`
		SessionID target.SessionID `json:"sessionId,omitzero"`
	}{event.Method, jsontext.Value(event.Params), event.SessionID})
	if err != nil {
		return err
	}
	for {
		s.mu.Lock()
		sockets := make([]*websocket.Conn, 0, len(s.sockets))
		for socket := range s.sockets {
			sockets = append(sockets, socket)
		}
		s.mu.Unlock()
		if len(sockets) > 0 {
			for _, socket := range sockets {
				if err := socket.Write(ctx, websocket.MessageText, data); err != nil {
					return err
				}
			}
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-s.ctx.Done():
			return s.ctx.Err()
		case <-s.connected:
		}
	}
}

// Close stops and joins the fixture before test cleanup if needed.
func (s *Server) Close(ctx context.Context) error {
	s.closeOnce.Do(func() {
		s.cancel()
		s.mu.Lock()
		s.closing = true
		sockets := make([]*websocket.Conn, 0, len(s.sockets))
		for socket := range s.sockets {
			sockets = append(sockets, socket)
		}
		s.mu.Unlock()
		for _, socket := range sockets {
			_ = socket.CloseNow()
		} // Fixture shutdown owns the terminal socket state.
		s.server.Close()
		s.wg.Wait()
		close(s.done)
	})
	select {
	case <-s.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (s *Server) respond(socket *websocket.Conn, request cdproto.Message) bool {
	exchange, err := s.script.take(string(request.Method), request.SessionID, request.Params)
	if err != nil {
		return false
	}
	reply := struct {
		ID        int64            `json:"id"`
		SessionID target.SessionID `json:"sessionId,omitzero"`
		Result    jsontext.Value   `json:"result,omitzero"`
		Error     *cdproto.Error   `json:"error,omitzero"`
	}{ID: request.ID, SessionID: request.SessionID}
	if exchange.Err != nil {
		var chrome *cdp.ProtocolError
		if !errors.As(exchange.Err, &chrome) {
			return false
		}
		reply.Error = &cdproto.Error{Code: chrome.Code, Message: chrome.Message}
	} else {
		result := exchange.Result
		if result == nil {
			result = struct{}{}
		}
		data, err := jsonv2.Marshal(result)
		if err != nil {
			s.t.Error(err)
			return false
		}
		reply.Result = jsontext.Value(data)
	}
	data, err := jsonv2.Marshal(reply)
	if err != nil {
		s.t.Error(err)
		return false
	}
	if err := socket.Write(s.ctx, websocket.MessageText, data); err != nil {
		return false
	}
	return true
}
