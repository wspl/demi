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

func newConversation(
	numbers tabs.NumberSource,
	launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error),
) *conversation {
	ctx, cancel := context.WithCancel(context.Background())
	c := &conversation{
		requests:    make(chan browserRequest, 64),
		done:        make(chan struct{}),
		released:    ctx,
		cancel:      cancel,
		changed:     make(chan struct{}),
		releaseDone: make(chan struct{}),
	}
	go c.run(numbers, launch)
	return c
}

func (c *conversation) admit() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.released.Err() != nil {
		return false
	}
	c.commands.Add(1)
	return true
}

func (c *conversation) publish(state lifecycle) {
	if c.state.Swap(uint32(state)) == uint32(state) {
		return
	}
	c.mu.Lock()
	close(c.changed)
	c.changed = make(chan struct{})
	c.mu.Unlock()
}

// Changed returns the next browser lifecycle notification.
func (c *conversation) Changed() <-chan struct{} {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.changed
}

// Released returns the conversation release notification.
func (c *conversation) Released() <-chan struct{} { return c.released.Done() }

func (c *conversation) ask(ctx context.Context, request browserRequest) (browserAnswer, error) {
	request.reply = make(chan browserAnswer, 1)
	select {
	case <-ctx.Done():
		return browserAnswer{}, ctx.Err()
	case <-c.done:
		return browserAnswer{}, &cdp.BrowserError{Kind: cdp.KindClosed}
	case c.requests <- request:
	}
	select {
	case <-ctx.Done():
		return browserAnswer{}, ctx.Err()
	case answer := <-request.reply:
		return answer, answer.err
	case <-c.done:
		select {
		case answer := <-request.reply:
			return answer, answer.err
		default:
			return browserAnswer{}, &cdp.BrowserError{Kind: cdp.KindClosed}
		}
	}
}

func (c *conversation) running(ctx context.Context, start *starting) (*tabs.Environment, *live.Hub, error) {
	answer, err := c.ask(ctx, browserRequest{start: start})
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

// Running waits for the current browser startup, if any.
func (c *conversation) Running(ctx context.Context) (*tabs.Environment, *live.Hub, error) {
	return c.running(ctx, nil)
}

func (c *conversation) retire(ctx context.Context, environment *tabs.Environment) error {
	_, err := c.ask(ctx, browserRequest{retire: environment})
	return err
}

func (c *conversation) release(ctx context.Context) error {
	c.mu.Lock()
	first := c.released.Err() == nil
	if first {
		c.cancel()
	}
	c.mu.Unlock()
	if first {
		// The releasing caller owns and joins this work even if its request ends.
		c.commands.Wait()
		_, c.cleanup = c.ask(context.WithoutCancel(ctx), browserRequest{release: true})
		<-c.done
		close(c.releaseDone)
	}
	<-c.releaseDone
	return c.cleanup
}

func (c *conversation) run(
	numbers tabs.NumberSource,
	launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error),
) {
	defer close(c.done)
	defer func() {
		c.publish(absent)
		c.mu.Lock()
		close(c.changed)
		c.mu.Unlock()
	}()
	var current *generation
	var cleanup error
	finish := func() {
		c.publish(closingBrowser)
		current.stop()
		<-current.done
		cleanup = current.cleanup
		current = nil
		c.publish(absent)
	}
	for {
		finished, ready, emptied, ended := c.generationSignals(current)
		select {
		case <-ready:
			if current.err == nil {
				c.publish(readyBrowser)
			} else {
				c.publish(closingBrowser)
			}
		case <-finished:
			finish()
		case <-emptied:
			finish()
		case <-ended:
			finish()
		case request := <-c.requests:
			if c.answerRequest(request, &current, &cleanup, finish, numbers, launch) {
				return
			}
		}
	}
}

// answerRequest returns whether release ends the owner loop. finish updates
// current and cleanup together before the request is answered.
func (c *conversation) answerRequest(
	request browserRequest,
	current **generation,
	cleanup *error,
	finish func(),
	numbers tabs.NumberSource,
	launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error),
) bool {
	if request.release {
		if *current != nil {
			finish()
		}
		request.reply <- browserAnswer{err: *cleanup}
		return true
	}
	if request.retire != nil {
		if *current != nil {
			select {
			case <-(*current).ready:
				if (*current).environment == request.retire {
					finish()
				}
			default:
			}
		}
		request.reply <- browserAnswer{err: *cleanup}
		return false
	}
	c.prepareGeneration(request, current, cleanup, finish, numbers, launch)
	return false
}

func (c *conversation) prepareGeneration(
	request browserRequest,
	current **generation,
	cleanup *error,
	finish func(),
	numbers tabs.NumberSource,
	launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error),
) {
	if c.released.Err() != nil {
		request.reply <- browserAnswer{err: &cdp.BrowserError{Kind: cdp.KindClosed}}
		return
	}
	if *current != nil {
		select {
		case <-(*current).done:
			finish()
		default:
			if request.start != nil && (*current).leaving() {
				finish()
			}
		}
	}

	if *cleanup != nil {
		request.reply <- browserAnswer{err: &cdp.BrowserError{
			Kind:    cdp.KindUnavailable,
			Message: "the browser's cleanup failed: " + (*cleanup).Error(),
			Cause:   *cleanup,
		}}
		return
	}
	if *current == nil && request.start != nil {
		ctx, stop := context.WithCancel(context.Background())
		*current = &generation{ready: make(chan struct{}), done: make(chan struct{}), stop: stop}
		c.publish(startingBrowser)
		go runGeneration(ctx, *current, request.start, numbers, launch)
	}
	answerGeneration(request, *current)
}

func answerGeneration(request browserRequest, current *generation) {
	if current != nil && request.start == nil {
		select {
		case <-current.ready:
			if environment := current.environment; environment != nil {
				select {
				case <-environment.Emptied():
					request.reply <- browserAnswer{}
					return
				case <-environment.Done():
					request.reply <- browserAnswer{err: environment.Failure()}
					return
				default:
				}
			}
		default:
		}
	}
	request.reply <- browserAnswer{generation: current}
}

func (c *conversation) generationSignals(current *generation) (
	finished, ready, emptied, ended <-chan struct{},
) {
	if current != nil {
		finished = current.done
		if lifecycle(c.state.Load()) == startingBrowser {
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
	return finished, ready, emptied, ended
}

func runGeneration(
	ctx context.Context,
	g *generation,
	start *starting,
	numbers tabs.NumberSource,
	launch func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error),
) {
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
