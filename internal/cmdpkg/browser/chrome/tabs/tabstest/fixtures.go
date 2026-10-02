package tabstest

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"testing"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// CaptureExtensionID is fixed by the capture extension manifest's public key.
const CaptureExtensionID = "ekadkclcinpnbbdeloemlmaimcklplko"

// Numbers supplies monotonically increasing tab numbers without a backend.
// Its zero value starts at one; calls are safe from concurrent creations.
type Numbers struct{}

// Next returns the next fixture number.
func (n *Numbers) Next(ctx context.Context) (uint64, error) {
	panic("not written: k-chrome-tabs")
}

// Launch starts real Chrome with isolated test directories and fixture numbers.
// Acceptance callers supply DEMI_TEST_CHROME in options; this function never
// discovers or downloads Chrome. Test cleanup closes and joins the environment.
func Launch(ctx context.Context, t testing.TB, options tabs.LaunchOptions) *tabs.Environment {
	panic("not written: k-chrome-tabs")
}

// Targets reads Chrome's own target list, including the capture extension.
func Targets(ctx context.Context, environment *tabs.Environment) ([]*target.Info, error) {
	panic("not written: k-chrome-tabs")
}
