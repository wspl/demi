package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"time"

	"github.com/wspl/demi/internal/core"
)

// Kept is how long a tool result's image or video stays in its block.
const Kept = 30 * 24 * time.Hour

// CacheLifetime is the longest time a vendor keeps a request in its cache.
const CacheLifetime = 24 * time.Hour

// Retirement identifies when the rule is applied and whether the conversation
// has been idle for Kept, permitting every expired tool medium to retire.
type Retirement struct {
	Now  core.Timestamp
	Idle bool
}

// RetiredBlock replaces the transcript block at Index with Value.
type RetiredBlock struct {
	Index int
	Value core.Block
}

// Retire returns replacements for expired tool images and videos, without
// changing blocks. A tool call must be older than Kept and either the conversation
// is idle or the call precedes a boundary older than CacheLifetime.
func Retire(blocks []core.Block, retirement Retirement) []RetiredBlock {
	panic("not written: a-transcript")
}
