package tabs

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// Tab is a registered top-level page. Sharing its pointer shares its gate and state.
// Closing it invalidates its references, debugging connections and owned work.
type Tab struct {
	environment *Environment
	session     *cdp.Session
	id          browserop.TabID
	createdBy   browserop.BrowserCreatedBy
	ctx         context.Context
	cancel      context.CancelCauseFunc
	gate        Gate
	loading     atomic.Bool
	console     *Console
	dialog      *DialogInput
	// mu protects task admission, lazy debug access and viewport publications.
	mu              sync.Mutex
	workers         sync.WaitGroup
	debug           *cdp.DebugOwner
	viewports       Viewports
	viewportChanged chan struct{}
}

// ID is the public conversation tab ID, never reused across environments.
func (t *Tab) ID() browserop.TabID {
	return t.id
}

// TargetID is Chrome's transport identity, not the public tab ID.
func (t *Tab) TargetID() target.ID {
	return t.session.TargetID()
}

// CreatedBy returns immutable diagnostic creation metadata, not authorization.
func (t *Tab) CreatedBy() browserop.BrowserCreatedBy {
	return t.createdBy
}

// Page supplies the renderer executor and its attached descendant frames.
func (t *Tab) Page() cdp.FrameTarget {
	return tabExecutor{t}
}

// Browser supplies a lifetime-checked browser executor without extending its life.
func (t *Tab) Browser() cdp.Executor {
	return t.environment.Browser()
}

// Context ends with the tab, preserving the environment's transport failure cause.
func (t *Tab) Context() context.Context {
	return t.ctx
}

// Done closes when the tab closes or its environment ends.
func (t *Tab) Done() <-chan struct{} {
	return t.ctx.Done()
}

// Operation shares the tab lifetime, invocation cancellation and absolute deadline.
// The caller defers the returned operation's Close.
func (t *Tab) Operation(ctx context.Context, deadline time.Time) *cdp.Operation {
	return cdp.OperationUntil(ctx, t.ctx, deadline)
}

// Gate supplies nonblocking command admission and its exclusively held session.
func (t *Tab) Gate() *Gate {
	return &t.gate
}

// Debug returns the lazily started debugging owner, whose lifetime the tab owns.
func (t *Tab) Debug() *cdp.DebugOwner {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.debug == nil {
		t.debug = cdp.StartDebug(t.ctx, t.environment.address, t.TargetID())
	}
	return t.debug
}

// DebuggingCallers returns other owners without starting debugging or calling Chrome.
func (t *Tab) DebuggingCallers(caller *uint64) []uint64 {
	t.mu.Lock()
	owner := t.debug
	t.mu.Unlock()
	if owner == nil {
		return nil
	}
	return owner.OtherCallers(caller)
}

// Console supplies the tab's bounded console collector.
func (t *Tab) Console() *Console {
	return t.console
}

// Dialog supplies the dialog owner and its deferred input releases.
func (t *Tab) Dialog() *DialogInput {
	return t.dialog
}

// Opened returns pages opened by this tab, registered or still being registered.
func (t *Tab) Opened(ctx context.Context) ([]browserop.TabID, error) {
	answer, err := t.environment.ask(ctx, registryRequest{kind: registryOpenedBy, target: t.TargetID()})
	return answer.ids, err
}

// Popups waits until this tab's observed popups have completed registration.
func (t *Tab) Popups(ctx context.Context) ([]browserop.TabID, error) {
	answer, err := t.environment.ask(ctx, registryRequest{kind: registryPopups, target: t.TargetID()})
	return answer.ids, err
}

// Close closes without beforeunload, cancelling detached navigation first.
// The environment owner must finish retirement when its last tab closes.
func (t *Tab) Close(ctx context.Context, timeout time.Duration) error {
	_, err := t.CloseRequest(ctx, timeout)
	return err
}

// CloseRequest closes the tab. emptied reports that it was the environment's
// last tab: the registry is sealed and the environment must retire.
func (t *Tab) CloseRequest(ctx context.Context, timeout time.Duration) (emptied bool, err error) {
	bounded, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	answer, err := t.environment.ask(
		bounded,
		registryRequest{kind: registryClose, target: t.TargetID(), deadline: time.Now().Add(timeout)},
	)
	if err != nil {
		return false, err
	}
	return answer.emptied, nil
}

// Input runs native input under the operation, reporting a blocking dialog.
// The callback uses its context and joins its work; deferred key/button releases
// are handed to Dialog, which retains them until the dialog is answered.
func (t *Tab) Input(ctx context.Context, operation *cdp.Operation, input func(context.Context) error) error {
	if t.dialog.IsOpen() {
		return &cdp.BrowserError{Kind: cdp.KindDialogBlocked}
	}
	return operation.Run(ctx, func(ctx context.Context) error {
		inputCtx, cancel := context.WithCancel(ctx)
		defer cancel()
		finished := make(chan error, 1)
		go func() { finished <- input(inputCtx) }()
		for {
			dialog, changed := t.dialog.Watch()
			if dialog != nil {
				cancel()
				<-finished
				return &cdp.BrowserError{Kind: cdp.KindDialogBlocked}
			}
			select {
			case err := <-finished:
				return err
			case <-ctx.Done():
				cancel()
				<-finished
				return ctx.Err()
			case <-changed:
			}
		}
	})
}

// Subscribe registers renderer events before the next page command. The caller
// closes the subscription; overflow is reported explicitly by its Next method.
func (t *Tab) Subscribe(methods ...string) (*cdp.Subscription, error) {
	return t.session.Subscribe(methods...)
}

// StartTask registers tab-lifetime work before starting it, or rejects it when
// closing. The callback must honor its context and release resources; tab closure
// cancels and joins it, and environment retirement also waits for it.
func (t *Tab) StartTask(work func(context.Context)) error {
	t.mu.Lock()
	if t.ctx.Err() != nil {
		t.mu.Unlock()
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	t.workers.Add(1)
	err := t.environment.StartTask(func(context.Context) {
		defer t.workers.Done()
		work(t.ctx)
	})
	if err != nil {
		t.workers.Done()
	}
	t.mu.Unlock()
	return err
}

// setUpTab observes page state before admitting commands.
func (e *Environment) setUpTab(
	ctx context.Context,
	id target.ID,
	public browserop.TabID,
	createdBy browserop.BrowserCreatedBy,
) (_ *Tab, err error) {
	session, err := e.connection.Attach(ctx, id)
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancelCause(e.ctx)
	view := unwatchedViewport()
	t := &Tab{
		environment:     e,
		session:         session,
		id:              public,
		createdBy:       createdBy,
		ctx:             lifetime,
		cancel:          cancel,
		viewports:       Viewports{Current: view, Web: view},
		viewportChanged: make(chan struct{}),
	}
	defer func() {
		if err != nil {
			cleanup, done := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
			defer done()
			err = cdp.AfterCleanup(err, t.shutdown(cleanup))
			err = cdp.AfterCleanup(err, session.Close(cleanup))
		}
	}()
	t.console, err = observeConsole(t)
	if err != nil {
		return nil, err
	}
	t.dialog, err = observeDialogs(t)
	if err != nil {
		return nil, err
	}
	renderer := protocol.WithExecutor(ctx, session)
	for _, enable := range []func(context.Context) error{
		page.Enable().Do,
		runtime.Enable().Do,
		network.Enable().Do,
		page.SetLifecycleEventsEnabled(
			true,
		).Do,
	} {
		if err := enable(renderer); err != nil {
			return nil, err
		}
	}
	if err := t.observeLoading(ctx); err != nil {
		return nil, err
	}
	if err := t.SetViewport(ctx, view); err != nil {
		return nil, err
	}
	return t, nil
}

// shutdown joins the tab's workers and debugging connections before tab cleanup ends.
func (t *Tab) shutdown(ctx context.Context) error {
	t.mu.Lock()
	t.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
	if t.debug == nil {
		t.debug = cdp.StartDebug(t.ctx, t.environment.address, t.TargetID())
	}
	owner := t.debug
	t.mu.Unlock()
	t.workers.Wait()
	if owner != nil {
		return owner.Close(ctx)
	}
	return nil
}

// closeTarget cancels detached navigation before stopping loading and destroying the page.
func (t *Tab) closeTarget(ctx context.Context) error {
	cleanup := t.shutdown(ctx)
	for {
		err := page.StopLoading().Do(protocol.WithExecutor(ctx, t.session))
		if err == nil {
			break
		}
		var chrome *cdp.ProtocolError
		// Chrome uses this vendor error while navigation changes renderer sessions.
		if !errors.As(err, &chrome) || chrome.Message != "Not attached to an active page" {
			return cdp.AfterCleanup(err, cleanup)
		}
		timer := time.NewTimer(20 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return cdp.AfterCleanup(ctx.Err(), cleanup)
		case <-timer.C:
			timer.Stop()
		}
	}
	err := target.CloseTarget(t.TargetID()).Do(protocol.WithExecutor(ctx, t.environment.connection))
	return cdp.AfterCleanup(err, cleanup)
}

type tabExecutor struct{ tab *Tab }

// Execute runs a renderer command within the tab lifetime and control deadline.
func (e tabExecutor) Execute(ctx context.Context, method string, params, result any) error {
	operation := cdp.NewOperation(ctx, e.tab.ctx, 30*time.Second)
	defer operation.Close()
	return operation.Run(
		ctx,
		func(ctx context.Context) error { return e.tab.session.Execute(ctx, method, params, result) },
	)
}

// TargetID identifies the tab renderer.
func (e tabExecutor) TargetID() target.ID { return e.tab.TargetID() }

// Related finds a renderer attached beneath the tab.
func (e tabExecutor) Related(ctx context.Context, id target.ID) (cdp.FrameTarget, error) {
	return e.tab.session.Related(ctx, id)
}

// Subscribe observes renderer events through the tab.
func (e tabExecutor) Subscribe(methods ...string) (*cdp.Subscription, error) {
	return e.tab.Subscribe(methods...)
}
