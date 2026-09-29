// Package usage meters provider responses and limits request admission.
package usage

import (
	"fmt"
	"sync"

	"github.com/wspl/demi/go/core"
)

type RateLimited struct{ Limit uint32 }

func (e *RateLimited) Error() string {
	return fmt.Sprintf("Provider request rate limit reached (%d per minute)", e.Limit)
}

type RateLimit struct {
	mu     sync.Mutex
	clock  core.Clock
	limit  uint32
	starts []int64
}

func NewRateLimit(clock core.Clock) *RateLimit { return RateLimitWithLimit(clock, 120) }
func RateLimitWithLimit(clock core.Clock, limit uint32) *RateLimit {
	return &RateLimit{clock: clock, limit: limit}
}
func (l *RateLimit) Take() *RateLimited {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := l.clock.Now().Millisecond()
	first := 0
	for first < len(l.starts) && now-l.starts[first] >= 60000 {
		first++
	}
	l.starts = append(l.starts[:0], l.starts[first:]...)
	if uint64(len(l.starts)) >= uint64(l.limit) {
		return &RateLimited{l.limit}
	}
	l.starts = append(l.starts, now)
	return nil
}
