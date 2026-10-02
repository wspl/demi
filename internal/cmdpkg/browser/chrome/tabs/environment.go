package tabs

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"sync/atomic"
	"time"

	"github.com/chromedp/cdproto/browser"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandwire"
)

// NumberSource supplies the next public tab number. cdp.TabNumbers implements it.
// A draw failure fails only the creation needing the number.
type NumberSource interface {
	Next(context.Context) (uint64, error)
}

// LaunchOptions identifies the installed pinned Chrome and the user's locale.
type LaunchOptions struct {
	Executable string
	Locale     commandwire.CommandLocale
}

// DirectoryBases are the Host's separate runtime and profile storage roots.
type DirectoryBases struct {
	Runtime  string
	Profiles string
}

// HostDirectories returns the platform's runtime and profile bases.
func HostDirectories() DirectoryBases {
	if runtime.GOOS == "windows" {
		return DirectoryBases{Runtime: os.TempDir(), Profiles: os.TempDir()}
	}
	profiles := os.Getenv("TMPDIR")
	if !filepath.IsAbs(profiles) {
		profiles = "/var/tmp"
	}
	return DirectoryBases{Runtime: "/tmp", Profiles: profiles}
}

// Environment owns Chrome, its process tree, profile, registry and joined workers.
// It does not own a conversation. Its caller must await Close even after cancellation.
type Environment struct {
	ctx             context.Context
	cancel          context.CancelCauseFunc
	connection      *cdp.Connection
	address         string
	transportCancel context.CancelFunc
	directories     *environmentDirectories
	process         *chromeProcess
	stopLogs        func()
	captures        *CaptureChannel
	closeCapture    func() error
	numbers         NumberSource
	requests        chan registryRequest
	snapshot        atomic.Pointer[Snapshot]
	snapshotChanged chan struct{} // Registry owner only.
	emptied         chan struct{}
	closed          chan struct{}
	// mu protects task admission, failure and change publication; never held across waits.
	mu       sync.Mutex
	workers  sync.WaitGroup
	failure  error
	revision uint64
	changed  chan struct{}
	cleanup  error // Published by closed.
}

// Launch starts the pinned Chrome with capture and CDP observation. Failed startup
// completes the same cleanup as Close before returning; no page actions are retried.
func Launch(ctx context.Context, options LaunchOptions, numbers NumberSource) (*Environment, error) {
	return launchIn(ctx, options, numbers, HostDirectories())
}

// Close stops admission, cancels work, retires Chrome, joins workers and removes
// the profile. Cleanup failures retain the profile and their underlying causes.
func (e *Environment) Close(ctx context.Context) error {
	e.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
	<-e.closed
	return cdp.AfterCleanup(ctx.Err(), e.cleanup)
}

// Browser returns the environment's lifetime-checked browser executor.
func (e *Environment) Browser() cdp.Executor {
	return environmentExecutor{e}
}

// Context ends with the environment and retains a transport failure as its cause.
func (e *Environment) Context() context.Context {
	return e.ctx
}

// Done closes when the environment ends; Close still must be awaited for cleanup.
func (e *Environment) Done() <-chan struct{} {
	return e.ctx.Done()
}

// Emptied closes when the registry seals after its final tab and batch are gone.
// The environment's owner must then await Close before acknowledging retirement.
func (e *Environment) Emptied() <-chan struct{} {
	return e.emptied
}

// Failure returns the browser's unsolicited transport failure, if any.
func (e *Environment) Failure() error {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.failure
}

// DownloadDirectory is where Chrome saves downloads until retirement.
func (e *Environment) DownloadDirectory() string {
	return filepath.Join(e.directories.profile, "downloads")
}

// SavedDownload allocates a fresh output path in the profile's download directory.
func (e *Environment) SavedDownload() (string, error) {
	name, err := cdp.Fresh("demi-download")
	if err != nil {
		return "", err
	}
	return filepath.Join(e.DownloadDirectory(), name), nil
}

// UploadDirectory holds files selected by the user until retirement.
func (e *Environment) UploadDirectory() string {
	return filepath.Join(e.directories.profile, "uploads")
}

// Captures returns the environment's capture extension connection.
func (e *Environment) Captures() *CaptureChannel {
	return e.captures
}

// Changes returns the current revision and its notification atomically. Reload
// after notification; titles, URLs, tabs and viewports may coalesce into one wake.
func (e *Environment) Changes() (uint64, <-chan struct{}) {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.revision, e.changed
}

// Open opens a URL as the root agent, waiting for DOMContentLoaded.
func (e *Environment) Open(ctx context.Context, url string, timeout time.Duration) (*Tab, error) {
	tab, _, err := e.OpenFor(ctx, url, 0, browserop.LoadDomContentLoaded, time.Now().Add(timeout))
	return tab, err
}

// OpenFor opens a distinct agent tab and returns its final URL after the chosen load.
func (e *Environment) OpenFor(ctx context.Context, url string, caller uint64, load browserop.Load, deadline time.Time) (*Tab, string, error) {
	if err := ValidateURL(url); err != nil {
		return nil, "", err
	}
	operation := cdp.OperationUntil(ctx, e.ctx, deadline)
	defer operation.Close()
	tab, err := e.create(operation.Context(), &browserop.BrowserCreatedByAgent{Number: caller})
	if err != nil {
		return nil, "", err
	}
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return nil, "", &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	final, err := tab.Navigate(ctx, &Visit{URL: url}, load, operation, &checkout.Session().References)
	return tab, final, err
}

// OpenUser creates a user tab, blank for nil URL, without waiting for navigation.
func (e *Environment) OpenUser(ctx context.Context, url *string, deadline time.Time) (*Tab, error) {
	if url != nil {
		if err := ValidateURL(*url); err != nil {
			return nil, err
		}
	}
	operation := cdp.OperationUntil(ctx, e.ctx, deadline)
	defer operation.Close()
	tab, err := e.create(operation.Context(), &browserop.BrowserCreatedByUser{})
	if err != nil {
		return nil, err
	}
	if url != nil {
		tab.detach(&Visit{URL: *url})
	}
	return tab, nil
}

// TemporaryTabs creates a batch and prevents final-tab retirement until it closes.
func (e *Environment) TemporaryTabs(ctx context.Context, caller uint64, count uint, deadline time.Time) (*TemporaryBatch, error) {
	operation := cdp.OperationUntil(ctx, e.ctx, deadline)
	defer operation.Close()
	if _, err := e.ask(operation.Context(), registryRequest{kind: registryHold}); err != nil {
		return nil, err
	}
	batch := &TemporaryBatch{environment: e}
	if err := operation.Context().Err(); err != nil {
		return nil, cdp.AfterCleanup(err, batch.Close(context.WithoutCancel(ctx), cdp.ControlTimeout))
	}
	for range count {
		tab, err := e.create(operation.Context(), &browserop.BrowserCreatedByTemporary{Number: caller})
		if err != nil {
			cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
			defer cancel()
			return nil, cdp.AfterCleanup(err, batch.Close(cleanup, cdp.ControlTimeout))
		}
		batch.tabs = append(batch.tabs, tab)
	}
	return batch, nil
}

// DebuggingCallers reads other agents' debugging ownership without browser calls.
func (e *Environment) DebuggingCallers(id browserop.TabID, caller *uint64) []uint64 {
	tab := e.Latest().Find(id)
	if tab == nil || tab.Context().Err() != nil {
		return nil
	}
	return tab.DebuggingCallers(caller)
}

// Tabs lists live tabs in creation order after pending registration settles.
func (e *Environment) Tabs(ctx context.Context, timeout time.Duration) ([]*Tab, error) {
	listed, err := e.Listed(ctx, timeout)
	if err != nil {
		return nil, err
	}
	result := make([]*Tab, 0, len(listed.Tabs))
	for _, entry := range listed.Tabs {
		result = append(result, entry.Tab)
	}
	return result, nil
}

// Listed reconciles event loss and waits for known registrations before listing.
func (e *Environment) Listed(ctx context.Context, timeout time.Duration) (*Snapshot, error) {
	operation := cdp.NewOperation(ctx, e.ctx, timeout)
	defer operation.Close()
	// Rust refreshes titles from Chrome, but tab lookup never does this IO.
	if _, err := e.ask(operation.Context(), registryRequest{kind: registryRetitle}); err != nil {
		return nil, err
	}
	for {
		snapshot := e.Latest()
		if !snapshot.Registering {
			return snapshot, nil
		}
		select {
		case <-operation.Context().Done():
			return nil, context.Cause(operation.Context())
		case <-snapshot.Changed:
		}
	}
}

// Latest returns the immutable registry publication without waiting on Chrome.
func (e *Environment) Latest() *Snapshot {
	return e.snapshot.Load()
}

// Tab finds a public ID in the snapshot, without waiting for another tab's command.
func (e *Environment) Tab(ctx context.Context, id browserop.TabID, timeout time.Duration) (*Tab, error) {
	operation := cdp.NewOperation(ctx, e.ctx, timeout)
	defer operation.Close()
	if err := operation.Context().Err(); err != nil {
		return nil, context.Cause(operation.Context())
	}
	tab := e.Latest().Find(id)
	if tab == nil || tab.ctx.Err() != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	return tab, nil
}

// TemporaryBatch holds temporary tabs and one environment-retirement lease.
type TemporaryBatch struct {
	environment *Environment
	tabs        []*Tab
	once        sync.Once
	cleanup     error
}

// Tabs returns the batch's tabs; callers must not modify the returned slice.
func (b *TemporaryBatch) Tabs() []*Tab {
	return b.tabs
}

// Close closes all batch tabs, releases its hold and waits for registry settlement.
func (b *TemporaryBatch) Close(ctx context.Context, timeout time.Duration) error {
	b.once.Do(func() {
		for _, tab := range b.tabs {
			err := tab.Close(ctx, timeout)
			var browserError *cdp.BrowserError
			if errors.As(err, &browserError) && browserError.Kind == cdp.KindTabNotFound {
				err = nil
			}
			b.cleanup = cdp.AfterCleanup(b.cleanup, err)
		}
		_, err := b.environment.ask(context.WithoutCancel(ctx), registryRequest{kind: registryRelease})
		if b.environment.ctx.Err() != nil {
			err = nil
		} // An ended registry holds no retirement leases.
		b.cleanup = cdp.AfterCleanup(b.cleanup, err)
	})
	return b.cleanup
}

// Subscribe registers browser-level events before the next command, including
// download events. The caller closes the subscription; overflow is explicit.
func (e *Environment) Subscribe(methods ...string) (*cdp.Subscription, error) {
	return e.connection.Subscribe(methods...)
}

// StartTask registers work before starting it, or rejects it once retirement
// begins. The callback must honor its environment context and release resources
// before returning. Close cancels and joins every registered callback.
func (e *Environment) StartTask(work func(context.Context)) error {
	e.mu.Lock()
	if e.ctx.Err() != nil {
		e.mu.Unlock()
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	e.workers.Add(1)
	e.mu.Unlock()
	go func() {
		defer e.workers.Done()
		work(e.ctx)
	}()
	return nil
}

// launchIn owns startup rollback as well as the launched environment's retirement.
func launchIn(ctx context.Context, options LaunchOptions, numbers NumberSource, bases DirectoryBases) (_ *Environment, err error) {
	if !filepath.IsAbs(options.Executable) {
		return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "Chrome executable must be absolute"}
	}
	directories, err := createDirectories(bases)
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancelCause(ctx)
	e := &Environment{ctx: lifetime, cancel: cancel, directories: directories, numbers: numbers, requests: make(chan registryRequest, 64), emptied: make(chan struct{}), closed: make(chan struct{}), changed: make(chan struct{})}
	e.snapshotChanged = make(chan struct{})
	e.snapshot.Store(&Snapshot{Tabs: []Listed{}, Changed: e.snapshotChanged})
	defer func() {
		if err != nil {
			cancel(err)
			e.retire()
			err = cdp.AfterCleanup(err, e.cleanup)
		}
	}()
	for _, directory := range []string{e.DownloadDirectory(), e.UploadDirectory()} {
		if err := os.Mkdir(directory, 0700); err != nil {
			return nil, err
		}
	}
	var captureAddress string
	e.captures, captureAddress, e.closeCapture, err = bindCapture(lifetime)
	if err != nil {
		return nil, err
	}
	command, err := configureLaunch(options, directories.profile, captureAddress)
	if err != nil {
		return nil, err
	}
	e.process, e.address, e.stopLogs, err = startChrome(ctx, command, directories)
	if err != nil {
		return nil, err
	}
	transport, transportCancel := context.WithCancel(context.Background())
	e.transportCancel = transportCancel
	// Dial's context owns its pump too. Startup cancellation only applies until
	// the connection has been established; retirement needs a live CDP pump.
	callbackDone := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(callbackDone)
		transportCancel()
	})
	e.connection, err = cdp.Dial(transport, e.address)
	if !stop() {
		<-callbackDone
	}
	if err != nil {
		return nil, err
	}
	setup, done := context.WithTimeout(lifetime, 30*time.Second)
	defer done()
	if err := browser.SetDownloadBehavior(browser.SetDownloadBehaviorBehaviorAllowAndName).WithDownloadPath(e.DownloadDirectory()).WithEventsEnabled(true).Do(protocol.WithExecutor(setup, e.connection)); err != nil {
		return nil, err
	}
	events, err := e.connection.SubscribeWithCapacity(256, "Target.targetCreated", "Target.targetInfoChanged", "Target.targetDestroyed")
	if err != nil {
		return nil, err
	}
	if err := target.SetDiscoverTargets(true).Do(protocol.WithExecutor(setup, e.connection)); err != nil {
		events.Close()
		return nil, err
	}
	if err := e.StartTask(func(ctx context.Context) { e.runRegistry(ctx, events) }); err != nil {
		events.Close()
		return nil, err
	}
	go func() {
		select {
		case <-e.ctx.Done():
		case <-e.emptied:
			e.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
		case <-e.connection.Done():
			failure := e.connection.Err()
			if failure == nil {
				failure = &cdp.BrowserError{Kind: cdp.KindConnection, Message: "browser connection ended"}
			}
			e.mu.Lock()
			if e.ctx.Err() == nil {
				e.failure = failure
			}
			e.mu.Unlock()
			e.cancel(failure)
		}
		e.retire()
	}()
	return e, nil
}

// retire stops admission before joining workers, then ends Chrome before its CDP pump.
func (e *Environment) retire() {
	defer close(e.closed)
	// Pair task admission with cancellation before Wait; no worker is added later.
	e.mu.Lock()
	e.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
	e.mu.Unlock()
	e.workers.Wait()
	if e.closeCapture != nil {
		e.cleanup = cdp.AfterCleanup(e.cleanup, e.closeCapture())
	}
	if e.connection != nil {
		ctx, cancel := context.WithTimeout(context.Background(), cdp.ControlTimeout)
		// Chrome can close the socket before replying. Process exit is authoritative.
		_ = browser.Close().Do(protocol.WithExecutor(ctx, e.connection))
		if e.process != nil && e.process.done != nil {
			select {
			case <-e.process.done:
			case <-ctx.Done():
			}
		}
		cancel()
	}
	var retired error
	if e.process != nil {
		ctx, cancel := context.WithTimeout(context.Background(), cdp.ControlTimeout)
		retired = e.process.retire(ctx)
		cancel()
	}
	if e.connection != nil {
		e.cleanup = cdp.AfterCleanup(e.cleanup, e.connection.Close(context.Background()))
	}
	if e.transportCancel != nil {
		e.transportCancel()
	}
	if e.stopLogs != nil {
		e.stopLogs()
	}
	e.cleanup = cdp.AfterCleanup(e.cleanup, e.directories.remove(context.Background(), retired))
}

// markChanged publishes the next browser-visible revision without blocking readers.
func (e *Environment) markChanged() {
	e.mu.Lock()
	old := e.changed
	e.revision++
	e.changed = make(chan struct{})
	e.mu.Unlock()
	close(old)
}

type environmentExecutor struct{ environment *Environment }

// Execute admits one bounded browser call while its environment is live.
func (b environmentExecutor) Execute(ctx context.Context, method string, params, result any) error {
	operation := cdp.NewOperation(ctx, b.environment.ctx, 30*time.Second)
	defer operation.Close()
	return operation.Run(ctx, func(ctx context.Context) error { return b.environment.connection.Execute(ctx, method, params, result) })
}
