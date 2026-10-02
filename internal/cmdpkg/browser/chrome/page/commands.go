package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Execute decodes and validates a tab command, then uses the same admission as the service.
// The returned JSON is encoded through the result contract, preserving Rust wire bytes.
func Execute(ctx context.Context, tab *tabs.Tab, name string, args json.RawMessage) (json.RawMessage, error) {
	panic("not written: k-chrome-page")
}

// Command executes a parsed tab command under one admission and absolute deadline.
// Results are encoded through their browserop contract; no second result union is declared.
func Command(ctx context.Context, tab *tabs.Tab, command browserop.Operation, deadline time.Time) (json.RawMessage, error) {
	panic("not written: k-chrome-page")
}

// CommandAdmitted keeps one browser-tab admission through a command and its attached artifact capture.
// The caller holds the tab checkout and passes its References until this call returns.
func CommandAdmitted(ctx context.Context, tab *tabs.Tab, command browserop.Operation, deadline time.Time, references *tabs.References) (json.RawMessage, error) {
	panic("not written: k-chrome-page")
}

// Metadata reads current tab information without acquiring the agent command gate.
func Metadata(ctx context.Context, tab *tabs.Tab, timeout time.Duration) (browserop.InfoResult, error) {
	panic("not written: k-chrome-page")
}
