package pagetest

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// ClickCSS clicks the element selector matches, for tests that drive a page.
func ClickCSS(ctx context.Context, tab *tabs.Tab, selector string, timeout time.Duration) error {
	_, err := page.Click(
		ctx,
		tab,
		browserop.ClickInput{
			Tab:           tab.ID(),
			BrowserTarget: browserop.BrowserTarget{BrowserQueryMatch: browserop.BrowserQueryMatch{CSS: &selector}},
		},
		time.Now().Add(timeout),
	)
	return err
}

// FillCSS fills the element selector matches, for tests that drive a page.
func FillCSS(ctx context.Context, tab *tabs.Tab, selector, text string, timeout time.Duration) error {
	_, err := page.Fill(
		ctx,
		tab,
		browserop.FillInput{
			Tab:           tab.ID(),
			BrowserTarget: browserop.BrowserTarget{BrowserQueryMatch: browserop.BrowserQueryMatch{CSS: &selector}},
			Text:          text,
		},
		time.Now().Add(timeout),
	)
	return err
}
