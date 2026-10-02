package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Value converts a typed result to the JSON value Render bounds, using the
// shared contract encoder so serde's escaping behavior is retained.
func Value(result any) (json.RawMessage, error) { panic("not written: k-chrome-cdp") }

// Resolve returns the absolute command output path against cwd. Empty paths
// and paths containing NUL are invalid input.
func Resolve(cwd, path string) (string, error) { panic("not written: k-chrome-cdp") }

// Preflight rejects an existing browser output before delivering page input.
func Preflight(ctx context.Context, cwd, output string, overwrite bool) (string, error) {
	panic("not written: k-chrome-cdp")
}

// SaveWithOverwrite publishes asset bytes with the explicit overwrite policy.
// The context supplies cancellation and the command's shared deadline.
func SaveWithOverwrite(ctx context.Context, cwd, output string, data []byte, overwrite bool) (string, error) {
	panic("not written: k-chrome-cdp")
}

// PublishFile atomically publishes a completed download without buffering it.
func PublishFile(ctx context.Context, cwd, output, source string, overwrite bool) (string, error) {
	panic("not written: k-chrome-cdp")
}

// Render bounds a result before success bytes escape, shrinking supported
// content and collections while preserving the first omitted stream position.
func Render(operation browserop.Operation, value json.RawMessage, asJSON bool) ([]byte, error) {
	panic("not written: k-chrome-cdp")
}

// RenderError renders failures with the progress and details of their JSON form.
func RenderError(code browserop.BrowserErrorCode, message string, details browserop.ErrorDetails) string {
	panic("not written: k-chrome-cdp")
}
