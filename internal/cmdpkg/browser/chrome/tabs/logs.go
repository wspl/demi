package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Console reads the tab-owned bounded console history. The tab joins its collector.
type Console struct{}

// Read filters and pages console entries, retaining independent cursors and gaps.
func (c *Console) Read(ctx context.Context, input browserop.LogsInput) (browserop.LogsResult, error) {
	panic("not written: k-chrome-tabs")
}
