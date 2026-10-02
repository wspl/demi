package cdp

import (
	"context"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// TabNumbers keeps a conversation's spare tab numbers and source. Share its
// pointer between callers. A nil source reports unavailable when a draw is needed.
type TabNumbers struct {
	source       *cmdsdk.Numbers
	conversation string
	mu           sync.Mutex
	spare        []uint64
}

// NewTabNumbers draws conversation tab numbers from the service's source.
func NewTabNumbers(source *cmdsdk.Numbers, conversation string) *TabNumbers {
	return &TabNumbers{source: source, conversation: conversation}
}

// Next returns the next number, drawing eight with Rust's 15-second deadline
// when no spare is at hand. A failed draw fails only this call.
func (n *TabNumbers) Next(ctx context.Context) (uint64, error) {
	n.mu.Lock()
	if len(n.spare) > 0 {
		number := n.spare[0]
		n.spare = n.spare[1:]
		n.mu.Unlock()
		return number, nil
	}
	n.mu.Unlock()
	if n.source == nil {
		return 0, &BrowserError{Kind: KindUnavailable, Message: "the service was given no tab numbers"}
	}
	draw, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	first, err := n.source.Draw(draw, n.conversation, commandwire.ServiceSequence("tab"), 8)
	if err != nil {
		message := "no tab numbers: " + err.Error()
		if draw.Err() == context.DeadlineExceeded {
			message = "the backend gave out no tab numbers in time"
		}
		return 0, &BrowserError{Kind: KindUnavailable, Message: message, Cause: err}
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	for i := uint64(0); i < 8; i++ {
		n.spare = append(n.spare, first+i)
	}
	number := n.spare[0]
	n.spare = n.spare[1:]
	return number, nil
}
