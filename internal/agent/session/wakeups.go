package session

import (
	"fmt"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// armLocked starts a session wakeup's duration only when its scheduling action ends.
func (s *Session) armLocked() {
	c := &s.core
	for i := range c.wakeups {
		if c.wakeups[i].DueAt != nil {
			continue
		}
		now, err := s.deps.Clock.Now().Time()
		if err != nil {
			s.eventLocked(&ErrorEvent{Report: ReportError{Message: err.Error()}})
			continue
		}
		due, err := core.TimestampFromTime(now.Add(time.Duration(c.wakeups[i].DurationMS) * time.Millisecond))
		if err != nil {
			s.eventLocked(&ErrorEvent{Report: ReportError{Message: err.Error()}})
			continue
		}
		c.wakeups[i].DueAt = &due
		c.dirty = true
	}
}

// wakeupDriver owns the single timer for the session's earliest due wakeup.
func (s *Session) wakeupDriver() {
	for {
		s.mu.Lock()
		changed := s.core.changed
		var next *store.ScheduledWakeup
		for _, w := range s.core.wakeups {
			if w.DueAt != nil && (next == nil || *w.DueAt < *next.DueAt) {
				next = new(w)
			}
		}
		now := s.deps.Clock.Now()
		s.mu.Unlock()
		if next == nil {
			select {
			case <-changed:
				continue
			case <-s.ctx.Done():
				return
			}
		}
		due, err := next.DueAt.Time()
		if err != nil {
			s.emit(&ErrorEvent{Report: ReportError{Message: fmt.Sprintf("yield wakeup: %v", err)}})
			return
		}
		current, err := now.Time()
		if err != nil {
			s.emit(&ErrorEvent{Report: ReportError{Message: err.Error()}})
			return
		}
		timer := time.NewTimer(max(0, due.Sub(current)))
		select {
		case <-s.ctx.Done():
			timer.Stop()
			return
		case <-changed:
			timer.Stop()
			continue
		case <-timer.C:
		}
		timer.Stop()
		s.acceptDueWakeups()
	}
}

func (s *Session) acceptDueWakeups() {
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		now := s.deps.Clock.Now()
		kept := []store.ScheduledWakeup{}
		for _, w := range c.wakeups {
			if w.DueAt != nil && *w.DueAt <= now {
				c.inputs = append(c.inputs, pendingInput{wakeup: new(w)})
				c.arrivals++
				c.dirty = true
			} else {
				kept = append(kept, w)
			}
		}
		c.wakeups = kept
		s.startNextLocked()
	})
}
