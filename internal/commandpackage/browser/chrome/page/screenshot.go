package page

import (
	"bytes"
	"context"
	"encoding/base64"
	"errors"
	"image"
	"image/color"
	"image/draw"
	"image/png"
	"math"
	"strconv"
	"strings"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

// Screenshot returns a PNG of what the tab shows, taken holding its operation lock.
func Screenshot(ctx context.Context, tab *tabs.Tab, timeout time.Duration) ([]byte, error) {
	return Capture(ctx, tab, false, nil, timeout)
}

// Capture returns a PNG of the viewport, whole page, or clip, holding the operation lock.
func Capture(ctx context.Context, tab *tabs.Tab, fullPage bool, clip *string, timeout time.Duration) ([]byte, error) {
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	operation := tab.Operation(ctx, time.Now().Add(timeout))
	defer operation.Close()
	var result []byte
	err := operation.Run(ctx, func(work context.Context) error {
		var err error
		result, err = ScreenshotBytes(work, tab, fullPage, clip)
		return err
	})
	return result, err
}

// ScreenshotBytes captures CSS pixels without changing the viewport.
// The caller holds the tab checkout and supplies the operation context.
func ScreenshotBytes(ctx context.Context, tab *tabs.Tab, fullPage bool, clip *string) ([]byte, error) {
	return screenshotBytes(ctx, tab.Page(), tab.Viewport(), fullPage, clip)
}

// screenshotBytes captures a CDP rectangle at the tab's CSS pixel scale.
func screenshotBytes(
	ctx context.Context,
	executor cdp.Executor,
	viewport browserproto.BrowserViewport,
	fullPage bool,
	clip *string,
) ([]byte, error) {
	if fullPage && clip != nil {
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "--full-page and --clip are mutually exclusive",
		}
	}
	rectangle := &chrome.Viewport{Scale: 1 / viewport.DevicePixelRatio}
	if clip != nil {
		if err := parseScreenshotClip(clip, rectangle); err != nil {
			return nil, err
		}
	} else {
		if err := screenshotViewport(ctx, executor, viewport, fullPage, rectangle); err != nil {
			return nil, err
		}
	}
	data, err := chrome.CaptureScreenshot().
		WithFormat(chrome.CaptureScreenshotFormatPng).
		WithFromSurface(true).
		WithCaptureBeyondViewport(fullPage || clip != nil).
		WithClip(rectangle).
		Do(protocol.WithExecutor(ctx, executor))
	var corrupt base64.CorruptInputError
	if errors.As(err, &corrupt) {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return data, err
}

// AnnotateProbe draws the probe markers onto PNG screenshot bytes.
func AnnotateProbe(data []byte, result browserproto.ProbeResult) ([]byte, error) {
	decoded, err := png.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	// Decode validated the PNG header; its color type distinguishes grayscale
	// with alpha, which Go expands to NRGBA. Both grayscale types are refused.
	if data[25] == 4 {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "Chrome screenshot must be RGB or RGBA"}
	}
	// Palette and 16-bit images are drawn as NRGBA; grayscale is refused.
	switch decoded.ColorModel() {
	case color.GrayModel, color.Gray16Model:
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "Chrome screenshot must be RGB or RGBA"}
	}
	pixels := image.NewNRGBA(decoded.Bounds())
	draw.Draw(pixels, pixels.Bounds(), decoded, decoded.Bounds().Min, draw.Src)
	width, height := pixels.Bounds().Dx(), pixels.Bounds().Dy()
	sx, sy := float64(width)/float64(result.Viewport.Width), float64(height)/float64(result.Viewport.Height)
	for _, node := range result.Matches {
		if node.Bounds == nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "probe bounds are missing"}
		}
		b := node.Bounds
		left, top := int(
			math.Min(math.Max(b.X*sx, 0), float64(width)),
		), int(
			math.Min(math.Max(b.Y*sy, 0), float64(height)),
		)
		right, bottom := int(
			math.Min(math.Max((b.X+b.Width)*sx, 0), float64(width)),
		), int(
			math.Min(math.Max((b.Y+b.Height)*sy, 0), float64(height)),
		)
		rectangles := []image.Rectangle{
			image.Rect(left, top, right, min(top+2, bottom)),
			image.Rect(left, max(bottom-2, top), right, bottom),
			image.Rect(left, top, min(left+2, right), bottom),
			image.Rect(max(right-2, left), top, right, bottom),
		}
		for _, r := range rectangles {
			for y := r.Min.Y; y < r.Max.Y; y++ {
				for x := r.Min.X; x < r.Max.X; x++ {
					offset := pixels.PixOffset(x, y)
					pixels.Pix[offset], pixels.Pix[offset+1], pixels.Pix[offset+2] = 255, 0, 80
				}
			}
		}
	}
	var output bytes.Buffer
	if err = png.Encode(&output, pixels); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return output.Bytes(), nil
}

func parseScreenshotClip(clip *string, rectangle *chrome.Viewport) error {
	parts := strings.Split(*clip, ",")
	if len(parts) != 4 {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "clip requires x,y,width,height"}
	}
	values := [4]float64{}
	for i, part := range parts {
		number, err := strconv.ParseFloat(part, 64)
		if err != nil {
			return &cdp.BrowserError{
				Kind:    cdp.KindConfiguration,
				Message: "clip requires x,y,width,height",
				Cause:   err,
			}
		}
		values[i] = number
	}
	for i, value := range values {
		if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || (i >= 2 && value == 0) {
			return &cdp.BrowserError{
				Kind:    cdp.KindConfiguration,
				Message: "clip requires finite nonnegative coordinates and positive dimensions",
			}
		}
	}
	rectangle.X, rectangle.Y, rectangle.Width, rectangle.Height = values[0], values[1], values[2], values[3]

	return nil
}

func screenshotViewport(
	ctx context.Context,
	executor cdp.Executor,
	viewport browserproto.BrowserViewport,
	fullPage bool,
	rectangle *chrome.Viewport,
) error {
	_, _, _, _, visible, size, err := chrome.GetLayoutMetrics().Do(protocol.WithExecutor(ctx, executor))
	if err != nil {
		return err
	}
	if fullPage {
		rectangle.X, rectangle.Y, rectangle.Width, rectangle.Height = size.X, size.Y, size.Width, size.Height
	} else {
		rectangle.X, rectangle.Y = visible.PageX, visible.PageY
		rectangle.Width, rectangle.Height = float64(viewport.Width), float64(viewport.Height)
	}
	return nil
}
