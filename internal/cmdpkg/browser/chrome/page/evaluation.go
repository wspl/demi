package page

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"encoding/json"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Evaluate evaluates expression read-only in the tab, holding its operation lock.
func Evaluate(ctx context.Context, tab *tabs.Tab, expression string, timeout time.Duration) (json.RawMessage, error) {
	panic("not written: k-chrome-page")
}
