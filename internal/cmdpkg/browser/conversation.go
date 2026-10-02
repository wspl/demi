package browser

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/live"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandwire"
)

type lifecycle uint32

const (
	absent lifecycle = iota
	startingBrowser
	readyBrowser
	closingBrowser
)

type starting struct {
	locale     commandwire.CommandLocale
	invocation string
}

type generation struct {
	ready       chan struct{}
	done        chan struct{}
	stop        context.CancelFunc
	environment *tabs.Environment
	hub         *live.Hub
	err         error
	cleanup     error
}

type browserRequest struct {
	start   *starting
	retire  *tabs.Environment
	release bool
	reply   chan browserAnswer
}

type browserAnswer struct {
	generation *generation
	err        error
}

// conversation owns the browser lifecycle; commands only read its publications.
type conversation struct {
	requests    chan browserRequest
	done        chan struct{}
	released    context.Context
	cancel      context.CancelFunc
	state       atomic.Uint32
	mu          sync.Mutex // Admission and change notification only; never held while waiting.
	changed     chan struct{}
	commands    sync.WaitGroup
	releaseDone chan struct{}
	cleanup     error
}

func newConversation(numbers tabs.NumberSource, launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error)) *conversation {
	ctx, cancel := context.WithCancel(context.Background())
	b := &conversation{requests: make(chan browserRequest, 64), done: make(chan struct{}), released: ctx, cancel: cancel, changed: make(chan struct{}), releaseDone: make(chan struct{})}
	go b.run(numbers, launch)
	return b
}

func (b *conversation) admit() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.released.Err() != nil {
		return false
	}
	b.commands.Add(1)
	return true
}

func (b *conversation) publish(state lifecycle) {
	if b.state.Swap(uint32(state)) == uint32(state) {
		return
	}
	b.mu.Lock()
	close(b.changed)
	b.changed = make(chan struct{})
	b.mu.Unlock()
}

func (b *conversation) Changed() <-chan struct{} {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.changed
}

func (b *conversation) Released() <-chan struct{} { return b.released.Done() }

func (b *conversation) ask(ctx context.Context, request browserRequest) (browserAnswer, error) {
	request.reply = make(chan browserAnswer, 1)
	select {
	case <-ctx.Done():
		return browserAnswer{}, ctx.Err()
	case <-b.done:
		return browserAnswer{}, &cdp.BrowserError{Kind: cdp.KindClosed}
	case b.requests <- request:
	}
	select {
	case <-ctx.Done():
		return browserAnswer{}, ctx.Err()
	case answer := <-request.reply:
		return answer, answer.err
	case <-b.done:
		select {
		case answer := <-request.reply:
			return answer, answer.err
		default:
			return browserAnswer{}, &cdp.BrowserError{Kind: cdp.KindClosed}
		}
	}
}

func (b *conversation) running(ctx context.Context, start *starting) (*tabs.Environment, *live.Hub, error) {
	answer, err := b.ask(ctx, browserRequest{start: start})
	if err != nil || answer.generation == nil {
		return nil, nil, err
	}
	g := answer.generation
	select {
	case <-ctx.Done():
		return nil, nil, ctx.Err()
	case <-g.ready:
		if g.err != nil {
			return nil, nil, &cdp.BrowserError{Kind: cdp.KindUnavailable, Message: g.err.Error(), Cause: g.err}
		}
		return g.environment, g.hub, nil
	}
}

func (b *conversation) Running(ctx context.Context) (*tabs.Environment, *live.Hub, error) {
	return b.running(ctx, nil)
}

func (b *conversation) retire(ctx context.Context, environment *tabs.Environment) error {
	_, err := b.ask(ctx, browserRequest{retire: environment})
	return err
}

func (b *conversation) release(ctx context.Context) error {
	b.mu.Lock()
	first := b.released.Err() == nil
	if first {
		b.cancel()
	}
	b.mu.Unlock()
	if first {
		// The releasing caller owns and joins this work even if its request ends.
		b.commands.Wait()
		_, b.cleanup = b.ask(context.WithoutCancel(ctx), browserRequest{release: true})
		<-b.done
		close(b.releaseDone)
	}
	<-b.releaseDone
	return b.cleanup
}

func (b *conversation) run(numbers tabs.NumberSource, launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error)) {
	defer close(b.done)
	defer func() {
		b.publish(absent)
		b.mu.Lock()
		close(b.changed)
		b.mu.Unlock()
	}()
	var current *generation
	var cleanup error
	finish := func() {
		b.publish(closingBrowser)
		current.stop()
		<-current.done
		cleanup = current.cleanup
		current = nil
		b.publish(absent)
	}
	for {
		var finished, ready <-chan struct{}
		var emptied, ended <-chan struct{}
		if current != nil {
			finished = current.done
			if lifecycle(b.state.Load()) == startingBrowser {
				ready = current.ready
			}
			select {
			case <-current.ready:
				if current.environment != nil {
					emptied = current.environment.Emptied()
					ended = current.environment.Done()
				}
			default:
			}
		}
		select {
		case <-ready:
			if current.err == nil {
				b.publish(readyBrowser)
			} else {
				b.publish(closingBrowser)
			}
		case <-finished:
			finish()
		case <-emptied:
			finish()
		case <-ended:
			finish()
		case request := <-b.requests:
			if request.release {
				if current != nil {
					finish()
				}
				request.reply <- browserAnswer{err: cleanup}
				return
			}
			if request.retire != nil {
				if current != nil {
					select {
					case <-current.ready:
						if current.environment == request.retire {
							finish()
						}
					default:
					}
				}
				request.reply <- browserAnswer{err: cleanup}
				continue
			}
			if b.released.Err() != nil {
				request.reply <- browserAnswer{err: &cdp.BrowserError{Kind: cdp.KindClosed}}
				continue
			}
			if current != nil {
				select {
				case <-current.done:
					finish()
				default:
					if request.start != nil && current.leaving() {
						finish()
					}
				}
			}

			if cleanup != nil {
				request.reply <- browserAnswer{err: &cdp.BrowserError{Kind: cdp.KindUnavailable, Message: "the browser's cleanup failed: " + cleanup.Error(), Cause: cleanup}}
				continue
			}
			if current == nil && request.start != nil {
				ctx, stop := context.WithCancel(context.Background())
				current = &generation{ready: make(chan struct{}), done: make(chan struct{}), stop: stop}
				b.publish(startingBrowser)
				go runGeneration(ctx, current, request.start, numbers, launch)
			}
			if current != nil && request.start == nil {
				select {
				case <-current.ready:
					if environment := current.environment; environment != nil {
						select {
						case <-environment.Emptied():
							request.reply <- browserAnswer{}
							continue
						case <-environment.Done():
							request.reply <- browserAnswer{err: environment.Failure()}
							continue
						default:
						}
					}
				default:
				}
			}
			request.reply <- browserAnswer{generation: current}
		}
	}
}

func runGeneration(ctx context.Context, g *generation, start *starting, numbers tabs.NumberSource, launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error)) {
	defer close(g.done)
	g.environment, g.hub, g.err = launch(ctx, start, numbers)
	close(g.ready)
	if g.environment == nil {
		// A failed startup can itself retain a profile; that fences future starts.
		var failure *cdp.BrowserError
		if errors.As(g.err, &failure) && (failure.Kind == cdp.KindCleanup || failure.Kind == cdp.KindProfileRetained) {
			g.cleanup = g.err
		}
		return
	}
	if g.err == nil {
		select {
		case <-ctx.Done():
		case <-g.environment.Done():
		case <-g.environment.Emptied():
		}
	}
	g.cleanup = g.environment.Close(context.WithoutCancel(ctx))
}

// leaving reports whether a generation must finish cleanup before another open.
func (g *generation) leaving() bool {
	select {
	case <-g.ready:
		if g.err != nil {
			return true
		}
		if g.environment != nil {
			select {
			case <-g.environment.Done():
				return true
			case <-g.environment.Emptied():
				return true
			default:
			}
		}
	default:
	}
	return false
}
