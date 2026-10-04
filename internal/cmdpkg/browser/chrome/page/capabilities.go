package page

import (
	"context"
	"encoding/json"
	"os"
	"runtime"

	"github.com/chromedp/cdproto/browser"
	protocol "github.com/chromedp/cdproto/cdp"
	js "github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Capabilities reports what the page command families offer.
func Capabilities(ctx context.Context, tab *tabs.Tab) ([]browserproto.Capability, error) {
	clipboard, err := ClipboardCapability(ctx, tab)
	if err != nil {
		return nil, err
	}
	return []browserproto.Capability{
		{ID: "accessibility", Available: true},
		{ID: "read-only-eval", Available: true},
		clipboard,
		{ID: "page-assets", Available: true},
		{ID: "cross-origin-frames", Available: true},
	}, nil
}

// ClipboardUnisolated returns why clipboard isolation is unverified, or nil where verified.
func ClipboardUnisolated() *string {
	_, wayland := os.LookupEnv("WAYLAND_DISPLAY")
	if runtime.GOOS == "darwin" || (runtime.GOOS == "linux" && !wayland) {
		return nil
	}
	reason := "clipboard isolation from the Host user's system clipboard has not been verified for this " +
		"platform configuration"
	return &reason
}

// ClipboardCapability publishes the pinned headless clipboard isolation policy for this document.
func ClipboardCapability(ctx context.Context, tab *tabs.Tab) (browserproto.Capability, error) {
	result := browserproto.Capability{ID: "clipboard", Reason: ClipboardUnisolated()}
	if result.Reason == nil {
		var err error
		result, err = clipboardDocumentReason(ctx, tab, result)
		if err != nil {
			return result, err
		}
	}
	result.Available = result.Reason == nil
	if result.Available {
		schema := json.RawMessage(
			`{"mimeTypes":["text/plain","text/html","image/png"],"help":"demi browser clipboard --help"}`,
		)
		result.Schema = &schema
	}
	return result, nil
}

// GrantClipboard lets pages use the browser's own clipboard.
func GrantClipboard(ctx context.Context, executor cdp.Executor) error {
	// The browser design requires this context-scoped permission grant; CDP deprecates it.
	// The pinned cdproto has removed GrantPermissions, so retain the exact
	// request required by the browser design through the executor.
	params := struct {
		Permissions []browser.PermissionType `json:"permissions"`
	}{[]browser.PermissionType{browser.PermissionTypeClipboardReadWrite, browser.PermissionTypeClipboardSanitizedWrite}}
	return executor.Execute(ctx, "Browser.grantPermissions", &params, nil)
}

func clipboardDocumentReason(
	ctx context.Context,
	tab *tabs.Tab,
	result browserproto.Capability,
) (browserproto.Capability, error) {
	object, exception, err := js.Evaluate("Boolean(navigator.clipboard && typeof ClipboardItem === 'function')").
		WithReturnByValue(true).
		Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		return result, err
	}
	if err = evaluationException(exception); err != nil {
		return result, err
	}
	var available bool
	if object == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "evaluation returned no JSON value"}
	}
	if err = json.Unmarshal(object.Value, &available); err != nil {
		return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if !available {
		reason := "the current document does not expose the Clipboard API; use a secure context"
		result.Reason = &reason
	}
	return result, nil
}
