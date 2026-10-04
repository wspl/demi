package tabstest

import (
	"context"
	"sync/atomic"
	"testing"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

// CaptureExtensionID is fixed by the capture extension manifest's public key.
const CaptureExtensionID = "ekadkclcinpnbbdeloemlmaimcklplko"

// Numbers supplies monotonically increasing tab numbers without a backend.
// Its zero value starts at one; calls are safe from concurrent creations.
type Numbers struct{ next atomic.Uint64 }

// Next returns the next fixture number.
func (n *Numbers) Next(ctx context.Context) (uint64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	return n.next.Add(1), nil
}

// Launch starts real Chrome with isolated test directories and fixture numbers.
// Acceptance callers supply DEMI_TEST_CHROME in options; this function never
// discovers or downloads Chrome. Test cleanup closes and joins the environment.
func Launch(ctx context.Context, t testing.TB, options tabs.LaunchOptions) *tabs.Environment {
	t.Helper()
	environment, err := tabs.Launch(ctx, options, &Numbers{})
	if err != nil {
		t.Fatalf("launch Chrome: %v", err)
	}
	t.Cleanup(func() {
		cleanup, cancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer cancel()
		if err := environment.Close(cleanup); err != nil {
			t.Errorf("retire Chrome: %v", err)
		}
	})
	return environment
}

// Targets reads Chrome's own target list, including the capture extension.
func Targets(ctx context.Context, environment *tabs.Environment) ([]*target.Info, error) {
	return target.GetTargets().Do(protocol.WithExecutor(ctx, environment.Browser()))
}
