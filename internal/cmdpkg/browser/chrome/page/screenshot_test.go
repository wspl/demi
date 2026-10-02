package page

import (
	"bytes"
	"image"
	"image/color"
	"image/png"
	"testing"

	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

func TestScreenshotClipKeepsCSSScale(t *testing.T) {
	clip := "1,2,30,40"
	params := chrome.CaptureScreenshot().WithFormat(chrome.CaptureScreenshotFormatPng).WithFromSurface(true).WithCaptureBeyondViewport(true).WithClip(&chrome.Viewport{X: 1, Y: 2, Width: 30, Height: 40, Scale: 0.5})
	executor := cdptest.NewExecutor(t, cdptest.Exchange{Method: chrome.CommandCaptureScreenshot, Params: params, Result: &chrome.CaptureScreenshotReturns{Data: "cG5n"}})
	data, err := screenshotBytes(t.Context(), executor, browserop.BrowserViewport{Width: 100, Height: 100, DevicePixelRatio: 2}, false, &clip)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "png" {
		t.Fatalf("data=%q", data)
	}
	for _, clip := range []string{"1,2,0,4", "NaN,0,1,1", "1,2,3"} {
		executor := cdptest.NewExecutor(t)
		if _, err := screenshotBytes(t.Context(), executor, browserop.BrowserViewport{}, false, &clip); err == nil || cdp.ErrorCode(err) != "invalid_input" {
			t.Fatalf("clip %q: %v", clip, err)
		}
	}
}

func TestProbeAnnotationPreservesInteriorAndAlpha(t *testing.T) {
	original := image.NewNRGBA(image.Rect(0, 0, 12, 12))
	for y := 0; y < 12; y++ {
		for x := 0; x < 12; x++ {
			original.SetNRGBA(x, y, color.NRGBA{R: 10, G: 20, B: 30, A: 123})
		}
	}
	var data bytes.Buffer
	if err := png.Encode(&data, original); err != nil {
		t.Fatal(err)
	}
	annotated, err := AnnotateProbe(data.Bytes(), browserop.ProbeResult{Viewport: browserop.BrowserViewport{Width: 6, Height: 6}, Matches: []browserop.BrowserNode{{Bounds: &browserop.Bounds{X: 1, Y: 1, Width: 4, Height: 4}}}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := png.Decode(bytes.NewReader(annotated))
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		x, y int
		want color.NRGBA
	}{{2, 2, color.NRGBA{R: 255, G: 0, B: 80, A: 123}}, {5, 5, color.NRGBA{R: 10, G: 20, B: 30, A: 123}}, {0, 0, color.NRGBA{R: 10, G: 20, B: 30, A: 123}}} {
		got := color.NRGBAModel.Convert(result.At(tc.x, tc.y))
		if got != tc.want {
			t.Fatalf("pixel %d,%d=%v, want %v", tc.x, tc.y, got, tc.want)
		}
	}
}
