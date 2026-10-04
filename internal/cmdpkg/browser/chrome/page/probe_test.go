package page_test

import (
	"bytes"
	"image"
	"image/color"
	"image/png"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
)

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
	annotated, err := page.AnnotateProbe(
		data.Bytes(),
		browserproto.ProbeResult{
			Viewport: browserproto.BrowserViewport{Width: 6, Height: 6},
			Matches:  []browserproto.BrowserNode{{Bounds: &browserproto.Bounds{X: 1, Y: 1, Width: 4, Height: 4}}},
		},
	)
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
	}{{
		2,
		2,
		color.NRGBA{
			R: 255,
			G: 0,
			B: 80,
			A: 123,
		},
	}, {
		5,
		5,
		color.NRGBA{
			R: 10,
			G: 20,
			B: 30,
			A: 123,
		},
	}, {
		0,
		0,
		color.NRGBA{
			R: 10,
			G: 20,
			B: 30,
			A: 123,
		},
	}} {
		got := color.NRGBAModel.Convert(result.At(tc.x, tc.y))
		if got != tc.want {
			t.Fatalf("pixel %d,%d=%v, want %v", tc.x, tc.y, got, tc.want)
		}
	}
}
