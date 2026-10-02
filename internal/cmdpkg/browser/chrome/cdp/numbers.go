package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
)

// TabNumbers keeps a conversation's spare tab numbers and source. Share its
// pointer between callers. A nil source reports unavailable when a draw is needed.
type TabNumbers struct{}

// NewTabNumbers draws conversation tab numbers from the service's source.
func NewTabNumbers(source *cmdsdk.Numbers, conversation string) *TabNumbers {
	panic("not written: k-chrome-cdp")
}

// Next returns the next number, drawing eight with Rust's 15-second deadline
// when no spare is at hand. A failed draw fails only this call.
func (n *TabNumbers) Next(ctx context.Context) (uint64, error) { panic("not written: k-chrome-cdp") }
