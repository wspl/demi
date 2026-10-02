// Package pagetest provides CSS-selector page interaction for Chrome acceptance tests.
package pagetest

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// ClickCSS clicks the element selector matches, for tests that drive a page.
func ClickCSS(ctx context.Context, tab *tabs.Tab, selector string, timeout time.Duration) error {
	panic("not written: k-chrome-page")
}

// FillCSS fills the element selector matches, for tests that drive a page.
func FillCSS(ctx context.Context, tab *tabs.Tab, selector, text string, timeout time.Duration) error {
	panic("not written: k-chrome-page")
}
