package machinestest

import (
	"context"
	"encoding/json"
	"sync"

	"github.com/wspl/demi/internal/machinewire"
)

// Service records calls and delegates their replies to a test script.
// Script must be safe for simultaneous calls and must join any work it starts.
type Service struct {
	// Script supplies the test reply after a call is recorded.
	Script func(context.Context, machinewire.Call) (json.RawMessage, error)
	mu     sync.Mutex
	calls  []machinewire.Call
}

// Handle records the call before invoking Script; a nil Script returns null.
func (s *Service) Handle(ctx context.Context, call machinewire.Call) (json.RawMessage, error) {
	s.mu.Lock()
	s.calls = append(s.calls, call)
	s.mu.Unlock()
	if s.Script != nil {
		return s.Script(ctx, call)
	}
	return json.RawMessage("null"), nil
}

// Calls returns a snapshot of the calls received.
func (s *Service) Calls() []machinewire.Call {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]machinewire.Call(nil), s.calls...)
}
