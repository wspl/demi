package host

import (
	"context"
	"errors"
	"sync"
)

// lifetime owns admitted Host operations until shutdown has joined every one.
// mu protects admission and the completion count, never a blocking operation.
type lifetime struct {
	ctx     context.Context
	cancel  context.CancelFunc
	mu      sync.Mutex
	active  int
	closing bool
	done    chan struct{}
}

func newLifetime(ctx context.Context) *lifetime {
	ctx, cancel := context.WithCancel(ctx)
	return &lifetime{ctx: ctx, cancel: cancel, done: make(chan struct{})}
}

// enter registers Host work before it can race with shutdown.
func (l *lifetime) enter(ctx context.Context) (context.Context, func(), error) {
	l.mu.Lock()
	if l.closing || l.ctx.Err() != nil {
		l.mu.Unlock()
		return nil, nil, errors.New("host connection closed")
	}
	l.active++
	l.mu.Unlock()
	work, cancel := context.WithCancel(ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(l.ctx, func() {
		cancel()
		close(stopped)
	})
	return work, func() {
		cancel()
		if !stop() {
			<-stopped
		}
		l.mu.Lock()
		l.active--
		if l.closing && l.active == 0 {
			close(l.done)
		}
		l.mu.Unlock()
	}, nil
}

func (l *lifetime) close(ctx context.Context) error {
	l.mu.Lock()
	if !l.closing {
		l.closing = true
		if l.active == 0 {
			close(l.done)
		}
	}
	l.mu.Unlock()
	l.cancel()
	select {
	case <-l.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// admit bounds finite Host work, while leaving pipe-paced transfers unbounded.
func admit(ctx context.Context, slots chan struct{}) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	select {
	case slots <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
