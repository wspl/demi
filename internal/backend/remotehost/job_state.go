package remotehost

import (
	"context"
	"io"
	"sync"
)

// jobState publishes runner output and a repeatable terminal result to its owner.
// Output remains lossless and does not block the connection router.
type jobState[E, C any] struct {
	mu      sync.Mutex // Protects output, terminal result and notification publication.
	output  []C
	end     *E
	changed chan struct{}
	hints   []runningHint
}
type runningHint struct{ id, text string }

// newJobState creates the output and completion publication for one runner operation.
func newJobState[E, C any]() *jobState[E, C] {
	return &jobState[E, C]{changed: make(chan struct{})}
}

// publish releases the job state before notifying its readers.
func (s *jobState[E, C]) publish() {
	previous := s.changed
	s.changed = make(chan struct{})
	s.mu.Unlock()
	close(previous)
}

// finish records the first terminal result and clears running command hints.
func (s *jobState[E, C]) finish(end E) {
	s.mu.Lock()
	if s.end != nil {
		s.mu.Unlock()
		return
	}
	s.end = new(end)
	s.hints = nil
	s.publish()
}

// hasEnded reports whether the terminal result is published.
func (s *jobState[E, C]) hasEnded() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.end != nil
}

// wait joins the runner operation's terminal publication.
func (s *jobState[E, C]) wait(ctx context.Context) (E, error) {
	for {
		s.mu.Lock()
		if s.end != nil {
			result := *s.end
			s.mu.Unlock()
			return result, nil
		}
		changed := s.changed
		s.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			var zero E
			return zero, ctx.Err()
		}
	}
}

// next drains ordered output before reporting terminal EOF.
func (s *jobState[E, C]) next(ctx context.Context) (C, error) {
	for {
		chunk, available, ended, changed := s.poll()
		if available {
			return chunk, nil
		}
		if ended {
			var zero C
			return zero, io.EOF
		}
		select {
		case <-changed:
		case <-ctx.Done():
			var zero C
			return zero, ctx.Err()
		}
	}
}

// hint keeps first-registration order while replacing each invocation's current guidance.
func (s *jobState[E, C]) hint(id string, text *string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.end != nil {
		return
	}
	for i, entry := range s.hints {
		if entry.id == id {
			if text == nil {
				s.hints = append(s.hints[:i], s.hints[i+1:]...)
			} else {
				s.hints[i].text = *text
			}
			return
		}
	}
	if text != nil {
		s.hints = append(s.hints, runningHint{id: id, text: *text})
	}
}

// runningHint returns the newest first-registered invocation still running.
func (s *jobState[E, C]) runningHint() *string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.hints) == 0 {
		return nil
	}
	return new(s.hints[len(s.hints)-1].text)
}

// poll takes one queued runner output chunk without waiting, returning its change publication.
func (s *jobState[E, C]) poll() (C, bool, bool, <-chan struct{}) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.output) > 0 {
		chunk := s.output[0]
		var zero C
		s.output[0] = zero
		s.output = s.output[1:]
		return chunk, true, false, s.changed
	}
	var zero C
	return zero, false, s.end != nil, s.changed
}

// push retains runner output until its owner reads it, without blocking other work.
func (s *jobState[E, C]) push(chunk C) {
	s.mu.Lock()
	if s.end != nil {
		s.mu.Unlock()
		return
	}
	s.output = append(s.output, chunk)
	s.publish()
}
