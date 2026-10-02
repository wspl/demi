package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// Upload attaches validated Host files to a file input.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func Upload(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], tab *tabs.Tab, input browserop.UploadInput, deadline time.Time) (browserop.UploadResult, error) {
	panic("not written: k-chrome-page")
}

// Download observes and saves the download triggered by one click.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func Download(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, input browserop.DownloadInput, deadline time.Time) (browserop.DownloadResult, error) {
	panic("not written: k-chrome-page")
}

// ClipboardRead reads the isolated browser clipboard.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ClipboardRead(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, input browserop.ClipboardReadInput, deadline time.Time) (browserop.ClipboardReadResult, error) {
	panic("not written: k-chrome-page")
}

// ClipboardWrite replaces the isolated browser clipboard with bounded invocation input.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ClipboardWrite(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, input browserop.ClipboardWriteInput, deadline time.Time) (browserop.ClipboardWriteResult, error) {
	panic("not written: k-chrome-page")
}

// AssetsList inventories resources already observed by the page.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func AssetsList(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], tab *tabs.Tab, input browserop.AssetsListInput, deadline time.Time) (browserop.AssetsListResult, error) {
	panic("not written: k-chrome-page")
}

// AssetsExport exports resources from the selected page inventory.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func AssetsExport(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], tab *tabs.Tab, input browserop.AssetsExportInput, deadline time.Time) (browserop.AssetsExportResult, error) {
	panic("not written: k-chrome-page")
}

// ContentFetch reads URLs through environment-owned temporary tabs.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ContentFetch(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, input browserop.ContentFetchInput, deadline time.Time) (browserop.ContentFetchResult, error) {
	panic("not written: k-chrome-page")
}
