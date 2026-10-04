package accounts

import (
	"sync"
	"time"

	"github.com/wspl/demi/internal/webapiproto"
)

const (
	lockAfter   = 5
	loginWindow = time.Minute
)

type failures struct {
	count       uint32
	lockedUntil time.Time
	forgetAt    time.Time
}

// LoginLimiter counts failures per email, including addresses with no account.
// Five failures within a minute of their predecessor lock the address for a
// minute. A success clears the count. Its zero value is ready for use.
type LoginLimiter struct {
	// mu protects failures; no work outside map operations runs under it.
	mu       sync.Mutex
	failures map[webapiproto.EmailAddress]failures
}

// NewLoginLimiter returns an empty process-local login limiter.
func NewLoginLimiter() *LoginLimiter { return &LoginLimiter{} }

// Locked reports whether even the right password must currently be refused.
func (l *LoginLimiter) Locked(email webapiproto.EmailAddress) bool {
	now := time.Now()
	l.mu.Lock()
	defer l.mu.Unlock()
	entry, ok := l.failures[email]
	if !ok {
		return false
	}
	if entry.lockedUntil.After(now) {
		return true
	}
	if !entry.forgetAt.After(now) {
		delete(l.failures, email)
	}
	return false
}

// Failed records a failed login and sweeps addresses whose window expired.
func (l *LoginLimiter) Failed(email webapiproto.EmailAddress) {
	now := time.Now()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.failures == nil {
		l.failures = make(map[webapiproto.EmailAddress]failures)
	}
	for address, entry := range l.failures {
		if !entry.forgetAt.After(now) {
			delete(l.failures, address)
		}
	}
	entry := l.failures[email]
	entry.count++
	if entry.count >= lockAfter {
		entry.count = 0
		entry.lockedUntil = now.Add(loginWindow)
	}
	entry.forgetAt = now.Add(loginWindow)
	l.failures[email] = entry
}

// Succeeded clears an address's remembered failures.
func (l *LoginLimiter) Succeeded(email webapiproto.EmailAddress) {
	l.mu.Lock()
	defer l.mu.Unlock()
	delete(l.failures, email)
}
