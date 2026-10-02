// Package cdptest supplies Chrome fixtures and scripted executors to browser tests.
package cdptest

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// EvaluateIn evaluates an expression over its own connection in a target that
// no public command addresses, such as the capture extension's worker.
// It closes its connection before returning, including on failure.
func EvaluateIn(ctx context.Context, address string, id target.ID, expression string) (json.RawMessage, error) {
	panic("not written: k-chrome-cdp")
}

// Exchange is one expected command and its scripted reply or failure.
// Params and Result use the same typed cdproto values as production callers.
type Exchange struct {
	Method    string
	SessionID target.SessionID
	Params    any
	Result    any
	Err       error
}

// Executor is a scripted cdproto executor. Test cleanup checks all exchanges
// were consumed; unexpected commands fail the test at the call boundary.
type Executor struct{}

// NewExecutor installs the script and its test-owned cleanup verification.
func NewExecutor(t testing.TB, exchanges ...Exchange) *Executor { panic("not written: k-chrome-cdp") }

// Execute checks a command against the next exchange and supplies its reply.
func (e *Executor) Execute(ctx context.Context, method string, params, result any) error {
	panic("not written: k-chrome-cdp")
}

// Server is a local scripted Chrome WebSocket fixture, with no Chrome process.
// Test cleanup closes sockets and joins all workers.
type Server struct{}

// NewServer starts a test-owned server on an ephemeral port.
func NewServer(t testing.TB, exchanges ...Exchange) *Server { panic("not written: k-chrome-cdp") }

// Address returns the fixture's browser WebSocket address.
func (s *Server) Address() string { panic("not written: k-chrome-cdp") }

// Emit sends an event after the test's preceding command or synchronization.
func (s *Server) Emit(ctx context.Context, event cdp.Event) error { panic("not written: k-chrome-cdp") }

// Close stops and joins the fixture before test cleanup if needed.
func (s *Server) Close(ctx context.Context) error { panic("not written: k-chrome-cdp") }
