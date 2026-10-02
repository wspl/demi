package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"time"

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
func HostDirectories() DirectoryBases { panic("not written: k-chrome-tabs") }

// Environment owns Chrome, its process tree, profile, registry and joined workers.
// It does not own a conversation. Its caller must await Close even after cancellation.
type Environment struct{}

// Launch starts the pinned Chrome with capture and CDP observation. Failed startup
// completes the same cleanup as Close before returning; no page actions are retried.
func Launch(ctx context.Context, options LaunchOptions, numbers NumberSource) (*Environment, error) {
	panic("not written: k-chrome-tabs")
}

// Close stops admission, cancels work, retires Chrome, joins workers and removes
// the profile. Cleanup failures retain the profile and their underlying causes.
func (e *Environment) Close(ctx context.Context) error { panic("not written: k-chrome-tabs") }

// Browser returns the environment's lifetime-checked browser executor.
func (e *Environment) Browser() cdp.Executor { panic("not written: k-chrome-tabs") }

// Context ends with the environment and retains a transport failure as its cause.
func (e *Environment) Context() context.Context { panic("not written: k-chrome-tabs") }

// Done closes when the environment ends; Close still must be awaited for cleanup.
func (e *Environment) Done() <-chan struct{} { panic("not written: k-chrome-tabs") }

// Emptied closes when the registry seals after its final tab and batch are gone.
// The environment's owner must then await Close before acknowledging retirement.
func (e *Environment) Emptied() <-chan struct{} { panic("not written: k-chrome-tabs") }

// Failure returns the browser's unsolicited transport failure, if any.
func (e *Environment) Failure() error { panic("not written: k-chrome-tabs") }

// DownloadDirectory is where Chrome saves downloads until retirement.
func (e *Environment) DownloadDirectory() string { panic("not written: k-chrome-tabs") }

// SavedDownload allocates a fresh output path in the profile's download directory.
func (e *Environment) SavedDownload() (string, error) { panic("not written: k-chrome-tabs") }

// UploadDirectory holds files selected by the user until retirement.
func (e *Environment) UploadDirectory() string { panic("not written: k-chrome-tabs") }

// Captures returns the environment's capture extension connection.
func (e *Environment) Captures() *CaptureChannel { panic("not written: k-chrome-tabs") }

// Changes returns the current revision and its notification atomically. Reload
// after notification; titles, URLs, tabs and viewports may coalesce into one wake.
func (e *Environment) Changes() (uint64, <-chan struct{}) { panic("not written: k-chrome-tabs") }

// Open opens a URL as the root agent, waiting for DOMContentLoaded.
func (e *Environment) Open(ctx context.Context, url string, timeout time.Duration) (*Tab, error) {
	panic("not written: k-chrome-tabs")
}

// OpenFor opens a distinct agent tab and returns its final URL after the chosen load.
func (e *Environment) OpenFor(ctx context.Context, url string, caller uint64, load browserop.Load, deadline time.Time) (*Tab, string, error) {
	panic("not written: k-chrome-tabs")
}

// OpenUser creates a user tab, blank for nil URL, without waiting for navigation.
func (e *Environment) OpenUser(ctx context.Context, url *string, deadline time.Time) (*Tab, error) {
	panic("not written: k-chrome-tabs")
}

// TemporaryTabs creates a batch and prevents final-tab retirement until it closes.
func (e *Environment) TemporaryTabs(ctx context.Context, caller uint64, count uint, deadline time.Time) (*TemporaryBatch, error) {
	panic("not written: k-chrome-tabs")
}

// DebuggingCallers reads other agents' debugging ownership without browser calls.
func (e *Environment) DebuggingCallers(id browserop.TabID, caller *uint64) []uint64 {
	panic("not written: k-chrome-tabs")
}

// Tabs lists live tabs in creation order after pending registration settles.
func (e *Environment) Tabs(ctx context.Context, timeout time.Duration) ([]*Tab, error) {
	panic("not written: k-chrome-tabs")
}

// Listed reconciles event loss and waits for known registrations before listing.
func (e *Environment) Listed(ctx context.Context, timeout time.Duration) (*Snapshot, error) {
	panic("not written: k-chrome-tabs")
}

// Latest returns the immutable registry publication without waiting on Chrome.
func (e *Environment) Latest() *Snapshot { panic("not written: k-chrome-tabs") }

// Tab finds a public ID in the snapshot, without waiting for another tab's command.
func (e *Environment) Tab(ctx context.Context, id browserop.TabID, timeout time.Duration) (*Tab, error) {
	panic("not written: k-chrome-tabs")
}

// TemporaryBatch holds temporary tabs and one environment-retirement lease.
type TemporaryBatch struct{}

// Tabs returns the batch's tabs; callers must not modify the returned slice.
func (b *TemporaryBatch) Tabs() []*Tab { panic("not written: k-chrome-tabs") }

// Close closes all batch tabs, releases its hold and waits for registry settlement.
func (b *TemporaryBatch) Close(ctx context.Context, timeout time.Duration) error {
	panic("not written: k-chrome-tabs")
}

// Subscribe registers browser-level events before the next command, including
// download events. The caller closes the subscription; overflow is explicit.
func (e *Environment) Subscribe(methods ...string) (*cdp.Subscription, error) {
	panic("not written: k-chrome-tabs")
}

// StartTask registers work before starting it, or rejects it once retirement
// begins. The callback must honor its environment context and release resources
// before returning. Close cancels and joins every registered callback.
func (e *Environment) StartTask(work func(context.Context)) error {
	panic("not written: k-chrome-tabs")
}
