package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// ToolSet is a page-declared set of tools and its version-bound opaque handle.
// This is state held by the tab, not a second wire contract.
type ToolSet struct {
	Handle  string
	Entries []browserop.WebmcpTool
}

// WebMCPState belongs to the calling tab and is held under its operation gate.
// A nil Document means no document has yet been observed.
type WebMCPState struct {
	Document *protocol.LoaderID
	Key      *string
	Tools    *ToolSet
}

// ExecuteWebMCP discovers or calls native page tools under the caller's existing
// tab admission, validating declarations and values with offline JSON Schema.
// The caller supplies the document executor and tab-owned state.
func ExecuteWebMCP(ctx context.Context, operation *Operation, executor Executor, state *WebMCPState, tab browserop.TabID, command browserop.Operation) (json.RawMessage, error) {
	panic("not written: k-chrome-cdp")
}
