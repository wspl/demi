package engine

import "context"

// beginLaunch registers a child start before it can borrow shell descriptors.
// Cancellation prevents new starts and joins these short-lived registrations
// before closing descriptors; no lock is held while a process starts.
func (e *execution) beginLaunch(ctx context.Context) (func(), error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	e.starting++
	return func() {
		e.mu.Lock()
		e.starting--
		if e.stopping && e.starting == 0 {
			close(e.launchesDone)
		}
		e.mu.Unlock()
	}, nil
}

func (e *execution) interrupt(ctx context.Context) func() {
	e.launchesDone = make(chan struct{})
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		e.mu.Lock()
		e.stopping = true
		if e.starting == 0 {
			close(e.launchesDone)
		}
		e.mu.Unlock()
		<-e.launchesDone
		if e.options.Interrupt != nil {
			e.options.Interrupt()
		}
		close(stopped)
	})
	return func() {
		if !stop() {
			<-stopped
		}
	}
}
