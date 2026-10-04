package page

import (
	"testing"

	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

func TestScreenshotClipKeepsCSSScale(t *testing.T) {
	clip := "1,2,30,40"
	params := chrome.CaptureScreenshot().
		WithFormat(chrome.CaptureScreenshotFormatPng).
		WithFromSurface(true).
		WithCaptureBeyondViewport(true).
		WithClip(&chrome.Viewport{X: 1, Y: 2, Width: 30, Height: 40, Scale: 0.5})
	executor := cdptest.NewExecutor(
		t,
		cdptest.Exchange{
			Method: chrome.CommandCaptureScreenshot,
			Params: params,
			Result: &chrome.CaptureScreenshotReturns{Data: "cG5n"},
		},
	)
	data, err := screenshotBytes(
		t.Context(),
		executor,
		browserop.BrowserViewport{Width: 100, Height: 100, DevicePixelRatio: 2},
		false,
		&clip,
	)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "png" {
		t.Fatalf("data=%q", data)
	}
	for _, clip := range []string{"1,2,0,4", "NaN,0,1,1", "1,2,3"} {
		executor := cdptest.NewExecutor(t)
		if _, err := screenshotBytes(
			t.Context(),
			executor,
			browserop.BrowserViewport{},
			false,
			&clip,
		); err == nil ||
			cdp.ErrorCode(err) != "invalid_input" {
			t.Fatalf("clip %q: %v", clip, err)
		}
	}
}
