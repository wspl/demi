package machinemanagertest

import (
	"context"
	"encoding/json"
	"sync"

	"github.com/wspl/demi/internal/machinemanagerproto"
)

// Service records calls and delegates their replies to a test script.
// Script must be safe for simultaneous calls and must join any work it starts.
type Service struct {
	// Script supplies the test reply after a call is recorded.
	Script func(context.Context, machinemanagerproto.Call) (json.RawMessage, error)
	mu     sync.Mutex
	calls  []machinemanagerproto.Call
}

// Handle records the call before invoking Script; a nil Script returns null.
func (s *Service) Handle(ctx context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
	s.mu.Lock()
	s.calls = append(s.calls, call)
	s.mu.Unlock()
	if s.Script != nil {
		return s.Script(ctx, call)
	}
	return json.RawMessage("null"), nil
}

// Calls returns a snapshot of the calls received.
func (s *Service) Calls() []machinemanagerproto.Call {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]machinemanagerproto.Call(nil), s.calls...)
}
