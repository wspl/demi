package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// TextTokens estimates text as UTF-8 bytes divided by four, rounded up.
func TextTokens(text string) uint64 { panic("not written: a-transcript") }

// BlockTokens estimates a block's text and media as request carries them.
func BlockTokens(block core.Block, request *RequestView) uint64 { panic("not written: a-transcript") }

// Estimate anchors on the latest reported usage after compaction and adds later
// blocks. Without a valid anchor it sums from the last compaction boundary.
// Usage above the model's nonzero context window is not a valid anchor.
func Estimate(request *RequestView) uint64 { panic("not written: a-transcript") }

// RequestSize is a request's weight as the vendor receives it.
type RequestSize struct {
	// Bytes counts base64 media and UTF-8 prompt and text, before vendor framing.
	Bytes uint64
	// Images counts image parts in both messages and tool results.
	Images uint64
}

// MeasureRequest measures already-replayed items together with the system prompt.
func MeasureRequest(systemPrompt string, items []provider.InferenceItem) RequestSize {
	panic("not written: a-transcript")
}
