package runners

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// PendingClaims holds runners waiting to be paired and each user's recent claims.
// Construct it with NewPendingClaims. Its methods are safe for concurrent use.
// Close releases every pending wait at backend shutdown.
type PendingClaims struct {
	mu                sync.Mutex // Protects pending runners and attempt windows; no IO under it.
	waiting           map[ClaimCode]*PendingRunner
	attempts          map[webapiproto.UserID][]time.Time
	attemptsPerMinute int
	closed            bool
}

// NewPendingClaims sets the number of attempts each user may make in one minute.
func NewPendingClaims(attemptsPerMinute int) *PendingClaims {
	return &PendingClaims{
		waiting:           make(map[ClaimCode]*PendingRunner),
		attempts:          make(map[webapiproto.UserID][]time.Time),
		attemptsPerMinute: attemptsPerMinute,
	}
}

// Attempt counts a claim attempt unless the user has exhausted the minute's
// allowance; then it returns false and counts nothing.
func (p *PendingClaims) Attempt(user webapiproto.UserID) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	now := time.Now()
	attempts := p.attempts[user]
	for len(attempts) > 0 && now.Sub(attempts[0]) >= time.Minute {
		attempts = attempts[1:]
	}
	p.attempts[user] = attempts
	if len(attempts) >= p.attemptsPerMinute {
		return false
	}
	p.attempts[user] = append(attempts, now)
	return true
}

// Take removes the runner under code so only this claim holds it, or returns nil.
// The caller must defer the returned runner's Release, including after Grant.
func (p *PendingClaims) Take(code ClaimCode) *PendingRunner {
	p.mu.Lock()
	runner := p.waiting[code]
	delete(p.waiting, code)
	p.mu.Unlock()
	return runner
}

// Register puts runner up for claiming under code; nil means shutdown has begun.
// The caller defers the wait's Release and Withdraw(code) when its socket leaves.
func (p *PendingClaims) Register(code ClaimCode, runner runnerproto.Info) *ClaimWait {
	wait := &ClaimWait{done: make(chan struct{})}
	p.mu.Lock()
	if p.closed {
		p.mu.Unlock()
		return nil
	}
	old := p.waiting[code]
	p.waiting[code] = &PendingRunner{Runner: runner, wait: wait}
	p.mu.Unlock()
	if old != nil {
		old.Release()
	}
	return wait
}

// Withdraw takes a code back when it expires or its runner goes away.
func (p *PendingClaims) Withdraw(code ClaimCode) {
	runner := p.Take(code)
	if runner != nil {
		runner.Release()
	}
}

// Close lets every waiting runner go and accepts no further registrations.
func (p *PendingClaims) Close() {
	p.mu.Lock()
	p.closed = true
	waiting := p.waiting
	p.waiting = nil
	p.mu.Unlock()
	for _, runner := range waiting {
		runner.Release()
	}
}

// ClaimWait owns the waiting runner's end of a claim. Do not copy it.
type ClaimWait struct {
	mu       sync.Mutex // Protects completion and transfer of the grant.
	done     chan struct{}
	finished bool
	grant    *ClaimGrant
}

// Wait receives the grant, or nil if the claim was withdrawn or abandoned.
// Context cancellation returns an error. A received grant must be released.
func (w *ClaimWait) Wait(ctx context.Context) (*ClaimGrant, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-w.done:
		w.mu.Lock()
		grant := w.grant
		w.grant = nil
		w.mu.Unlock()
		return grant, nil
	}
}

// Release idempotently abandons the runner's wait and any undelivered grant.
func (w *ClaimWait) Release() {
	w.complete(nil)
	w.mu.Lock()
	grant := w.grant
	w.grant = nil
	w.mu.Unlock()
	if grant != nil {
		grant.Release()
	}
}

// PendingRunner is a waiting runner taken by one claim. Do not copy it.
type PendingRunner struct {
	once sync.Once
	wait *ClaimWait
	// Runner is the information reported by the waiting runner.
	Runner runnerproto.Info
}

// ErrRunnerLeft means the runner went away before it bound its socket.
var ErrRunnerLeft = errors.New("the runner left before it bound its socket")

// Grant hands the runner its device and token and waits until the runner
// binds its socket. It returns ErrRunnerLeft if the runner leaves first and
// ctx.Err() if ctx ends first; either way the claim is abandoned.
func (p *PendingRunner) Grant(
	ctx context.Context,
	device database.DeviceRecord,
	token runnerproto.DeviceToken,
) (webapiproto.DeviceDTO, error) {
	answer := &claimAnswer{done: make(chan struct{})}
	delivered := false
	p.once.Do(func() {
		delivered = true
		p.wait.complete(&ClaimGrant{Device: device, Token: token, answer: answer})
	})
	if !delivered {
		answer.Release()
	}
	defer answer.Release()
	select {
	case <-ctx.Done():
		return webapiproto.DeviceDTO{}, ctx.Err()
	case <-answer.done:
	}
	if answer.device == nil {
		return webapiproto.DeviceDTO{}, ErrRunnerLeft
	}
	return *answer.device, nil
}

// Release idempotently abandons an ungranted claim; it does nothing after Grant.
func (p *PendingRunner) Release() {
	p.once.Do(func() {
		p.wait.complete(nil)
	})
}

// ClaimGrant hands a runner its device and private token, and owns the reply
// to the claimant. The runner defers Release and calls Bound after socket binding.
type ClaimGrant struct {
	answer *claimAnswer
	// Device is the device created by the claim.
	Device database.DeviceRecord
	// Token is the credential only the runner receives.
	Token runnerproto.DeviceToken
}

// Bound completes the claimant's answer with the bound device, without waiting.
func (g *ClaimGrant) Bound(device webapiproto.DeviceDTO) {
	g.answer.once.Do(func() {
		g.answer.device = &device
		close(g.answer.done)
	})
}

// Release idempotently abandons an unanswered grant, waking its claimant.
func (g *ClaimGrant) Release() {
	g.answer.Release()
}

// claimAnswer owns the claimant's wait for the runner to bind its socket.
type claimAnswer struct {
	once   sync.Once
	done   chan struct{}
	device *webapiproto.DeviceDTO
}

// Release idempotently abandons the claimant's answer without blocking the runner.
func (a *claimAnswer) Release() {
	a.once.Do(func() {
		close(a.done)
	})
}

// complete transfers a claim grant to the waiting runner, or abandons it if the
// runner already left. Notification and grant release happen outside the mutex.
func (w *ClaimWait) complete(grant *ClaimGrant) {
	w.mu.Lock()
	if w.finished {
		w.mu.Unlock()
		if grant != nil {
			grant.Release()
		}
		return
	}
	w.finished = true
	w.grant = grant
	w.mu.Unlock()
	close(w.done)
}
