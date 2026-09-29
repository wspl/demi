package auth

import (
	"sync"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type failures struct {
	count       uint32
	lockedUntil *int64
	forgetAt    int64
}
type LoginLimiter struct {
	mu       sync.Mutex
	clock    core.Clock
	failures map[webapi.EmailAddress]failures
}

func NewLoginLimiter(clock core.Clock) *LoginLimiter {
	return &LoginLimiter{clock: clock, failures: make(map[webapi.EmailAddress]failures)}
}
func (l *LoginLimiter) Locked(email webapi.EmailAddress) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := l.clock.Now().Millisecond()
	entry, ok := l.failures[email]
	if !ok {
		return false
	}
	if entry.lockedUntil != nil && *entry.lockedUntil > now {
		return true
	}
	if entry.forgetAt <= now {
		delete(l.failures, email)
	}
	return false
}
func (l *LoginLimiter) Failed(email webapi.EmailAddress) {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := l.clock.Now().Millisecond()
	for key, entry := range l.failures {
		if entry.forgetAt <= now {
			delete(l.failures, key)
		}
	}
	entry := l.failures[email]
	entry.count++
	if entry.count >= 5 {
		entry.count = 0
		until := now + 60000
		entry.lockedUntil = &until
	}
	entry.forgetAt = now + 60000
	l.failures[email] = entry
}
func (l *LoginLimiter) Succeeded(email webapi.EmailAddress) {
	l.mu.Lock()
	defer l.mu.Unlock()
	delete(l.failures, email)
}
