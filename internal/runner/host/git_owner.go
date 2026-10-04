package host

import (
	"context"
	"sync"
	"time"

	"github.com/wspl/demi/internal/runnerproto"
)

type gitAnswer struct {
	changes runnerproto.GitChanges
	err     error
}
type gitRequest struct {
	root   string
	ctx    context.Context
	answer chan gitAnswer
}
type gitCompleted struct {
	root   string
	answer gitAnswer
}
type gitRoot struct {
	state           *treeState
	last            time.Time
	current, queued []gitRequest
}

// gitOwner serializes baseline ownership, never filesystem work. Its bounded
// request queue waits for space; service admission bounds producers to eight.
type gitOwner struct {
	ctx      context.Context
	cancel   context.CancelFunc
	requests chan gitRequest
	done     chan struct{}
}

func newGitOwner(ctx context.Context, slots chan struct{}, limit int) *gitOwner {
	ctx, cancel := context.WithCancel(ctx)
	owner := &gitOwner{ctx: ctx, cancel: cancel, requests: make(chan gitRequest, 64), done: make(chan struct{})}
	go owner.run(slots, limit)
	return owner
}

func (o *gitOwner) close(ctx context.Context) error {
	o.cancel()
	select {
	case <-o.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (o *gitOwner) changes(ctx context.Context, root string) (runnerproto.GitChanges, error) {
	request := gitRequest{root: root, ctx: ctx, answer: make(chan gitAnswer, 1)}
	select {
	case <-ctx.Done():
		return runnerproto.GitChanges{}, ctx.Err()
	case <-o.ctx.Done():
		return runnerproto.GitChanges{}, o.ctx.Err()
	case o.requests <- request:
	}
	select {
	case <-ctx.Done():
		return runnerproto.GitChanges{}, ctx.Err()
	case <-o.ctx.Done():
		return runnerproto.GitChanges{}, o.ctx.Err()
	case answer := <-request.answer:
		return answer.changes, answer.err
	}
}

func (o *gitOwner) run(slots chan struct{}, limit int) {
	defer close(o.done)
	ticker := time.NewTicker(time.Minute)
	defer ticker.Stop()
	roots := make(map[string]*gitRoot)
	completed := make(chan gitCompleted, 8)
	var workers sync.WaitGroup
	defer func() {
		workers.Wait()
		for _, root := range roots {
			root.state.stop()
		}
	}()
	start := func(path string, root *gitRoot) {
		o.startRoot(path, root, &workers, completed, slots, limit)
	}
	expire := func() { expireRoots(roots) }
	accept := func(request gitRequest) bool {
		return o.acceptRoot(request, roots, expire, start)
	}
	var waiting []gitRequest
	for {
		select {
		case <-o.ctx.Done():
			return
		case <-ticker.C:
			expire()
		case request := <-o.requests:
			if !accept(request) {
				waiting = append(waiting, request)
			}
		case result := <-completed:
			root := roots[result.root]
			for _, request := range root.current {
				request.answer <- result.answer
			}
			root.current = nil
			start(result.root, root)
			pending := waiting[:0]
			for _, request := range waiting {
				if !accept(request) {
					pending = append(pending, request)
				}
			}
			waiting = pending
		}
	}
}

func (o *gitOwner) startRoot(
	path string,
	root *gitRoot,
	workers *sync.WaitGroup,
	completed chan<- gitCompleted,
	slots chan struct{},
	limit int,
) {
	if len(root.current) > 0 {
		return
	}
	live := root.queued[:0]
	for _, request := range root.queued {
		if request.ctx.Err() == nil {
			live = append(live, request)
		}
	}
	root.queued = nil
	if len(live) == 0 {
		return
	}
	root.current = live
	workers.Go(func() {
		changes, err := retryChanges(o.ctx, root.state, path, slots, limit)
		select {
		case completed <- gitCompleted{path, gitAnswer{changes, err}}:
		case <-o.ctx.Done():
		}
	})
}

func expireRoots(roots map[string]*gitRoot) {
	for path, root := range roots {
		if len(root.current) == 0 && len(root.queued) == 0 && time.Since(root.last) >= 15*time.Minute {
			root.state.stop()
			delete(roots, path)
		}
	}
}

func (o *gitOwner) acceptRoot(
	request gitRequest,
	roots map[string]*gitRoot,
	expire func(),
	start func(string, *gitRoot),
) bool {
	if request.ctx.Err() != nil {
		return true
	}
	expire()
	root := roots[request.root]
	if root == nil {
		if len(roots) >= 8 {
			oldest := oldestIdleRoot(roots)
			if oldest == "" {
				return false
			}
			roots[oldest].state.stop()
			delete(roots, oldest)
		}
		ctx, cancel := context.WithCancel(o.ctx)
		root = &gitRoot{state: &treeState{ctx: ctx, cancel: cancel, touched: make(map[string]bool)}}
		roots[request.root] = root
	}
	root.last = time.Now()
	root.queued = append(root.queued, request)
	start(request.root, root)
	return true
}

func oldestIdleRoot(roots map[string]*gitRoot) (oldest string) {
	for path, candidate := range roots {
		if len(candidate.current) == 0 && (oldest == "" || candidate.last.Before(roots[oldest].last)) {
			oldest = path
		}
	}
	return oldest
}
