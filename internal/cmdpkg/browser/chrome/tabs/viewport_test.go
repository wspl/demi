package tabs

import (
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

func TestRatioStaysWithinCaptureEncoding(t *testing.T) {
	fractional := 1.5
	if runtime.GOOS == "darwin" {
		fractional = 2
	}
	for _, test := range []struct {
		ratio         float64
		width, height uint32
		want          float64
	}{
		{1.5, 1280, 720, fractional}, {2, 1440, 900, 2}, {0.8, 1440, 900, 1}, {3, 390, 844, 3},
	} {
		if got := RatioFor(test.ratio, test.width, test.height); got != test.want {
			t.Fatalf("ratio for %+v = %g", test, got)
		}
	}
	limited := RatioFor(3, 1800, 1000)
	if limited*1800 > 4096 || limited < 2 {
		t.Fatal(limited)
	}
}

func TestPictureHasEvenDevicePixels(t *testing.T) {
	viewport := browserop.BrowserViewport{
		Mode:             browserop.ViewportModeWeb,
		Width:            701,
		Height:           401,
		DevicePixelRatio: 1.5,
	}
	for _, test := range []struct {
		scale         float64
		width, height uint32
	}{{1, 1052, 602}, {0.5, 526, 302}} {
		width, height := Pixels(viewport, test.scale)
		if width != test.width || height != test.height {
			t.Fatalf("scale %g: %d x %d", test.scale, width, height)
		}
	}
}
