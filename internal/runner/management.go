package runner

import (
	"context"
	"crypto/subtle"
	"errors"
	"sync"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/jobs"
)

const manageOperation = "manage"

// management publishes installation status and owns the drain notification.
type management struct {
	// mu protects the status snapshot; no IO runs while held.
	mu       sync.Mutex
	status   managementStatus
	secret   string
	draining chan struct{}
	once     sync.Once
}

func newManagement(secret, release string) *management {
	return &management{
		secret:   secret,
		status:   managementStatus{Release: release, Phase: connecting},
		draining: make(chan struct{}),
	}
}

func (m *management) snapshot() managementStatus {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.status
}

func (m *management) setPhase(p phase) {
	m.mu.Lock()
	m.status.Phase = p
	m.mu.Unlock()
}

func (m *management) setJobs(n int) {
	m.mu.Lock()
	m.status.Jobs = uint64(n)
	m.mu.Unlock()
}

func (m *management) drain() {
	m.once.Do(func() {
		m.mu.Lock()
		m.status.Draining = true
		m.mu.Unlock()
		close(m.draining)
	})
}

// endpoint serves management beside declared commands on the private socket.
type endpoint struct {
	dispatcher *jobs.Dispatcher
	management *management
}

func (e *endpoint) Operations() []string {
	return append(e.dispatcher.Operations(), manageOperation)
}

func (e *endpoint) Invoke(
	ctx context.Context,
	inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
) (commandwire.Completion, error) {
	if inv.Request.Operation != manageOperation {
		return e.dispatcher.Invoke(ctx, inv)
	}
	err := e.answer(ctx, inv)
	return jobs.Reported(ctx, commandwire.Completion{}, err, inv.Output)
}

func (e *endpoint) answer(ctx context.Context, inv cmdsdk.InvocationContext[commandwire.LocalInvocation]) error {
	request, err := decodeManagementRequest(inv.Request.Args)
	if err != nil {
		return err
	}
	if subtle.ConstantTimeCompare([]byte(request.Secret), []byte(e.management.secret)) != 1 {
		return errors.New("invalid management secret")
	}
	if request.Action == drainAction {
		e.management.drain()
	}
	data, err := e.management.snapshot().MarshalJSON()
	if err != nil {
		return err
	}
	return inv.Output.Stdout(ctx, data)
}
