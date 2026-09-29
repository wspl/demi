package backend

import (
	"context"
	"sync"
)

// TaskGroup is a set of goroutines a shard owns, the Rust's TaskTracker: each
// runs with the group's context, which ends when the shard starts closing or
// the group closes; a closed group starts no more, and wait returns once
// every one has returned.
type TaskGroup struct {
	ctx     context.Context
	cancel  context.CancelFunc
	mu      sync.Mutex
	closed  bool
	running sync.WaitGroup
}

func newTaskGroup(parent context.Context) *TaskGroup {
	ctx, cancel := context.WithCancel(parent)
	return &TaskGroup{ctx: ctx, cancel: cancel}
}

// Go starts work on a goroutine of the group and reports whether it did: a
// closed group refuses it. The work reaches its shard's state only through a
// ShardRef.
func (g *TaskGroup) Go(work func(ctx context.Context)) bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.closed {
		return false
	}
	g.running.Go(func() { work(g.ctx) })
	return true
}

// close ends the group's context and refuses new goroutines.
func (g *TaskGroup) close() {
	g.mu.Lock()
	g.closed = true
	g.mu.Unlock()
	g.cancel()
}

// wait returns once every goroutine of the group has returned.
func (g *TaskGroup) wait() { g.running.Wait() }
