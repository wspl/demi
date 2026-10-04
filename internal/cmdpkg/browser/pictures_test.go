//go:build acceptance

package browser

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/contract"
)

// decodedPicture uses Chrome's WebCodecs decoder, as the live page does.
func (f *browserFixture) decodedPicture(t *testing.T, tab browserproto.TabID, frame []byte) image.Image {
	t.Helper()
	script, err := os.ReadFile("testdata/decode-picture.js")
	if err != nil {
		t.Fatal(err)
	}
	expression := "(" + string(
		script,
	) + ")(" + string(
		mustBrowserValue(t, base64.StdEncoding.EncodeToString(frame)),
	) + "," + string(
		mustBrowserValue(t, browserproto.VideoCodec),
	) + ")"
	result := f.mutate(t, tab, expression)
	length, err := contract.Decode[int](observedField(t, result, "result", "result", "value"))
	if err != nil {
		t.Fatalf("decode picture %s: %v", result, err)
	}
	encoded := ""
	for len(encoded) < length {
		expression := "globalThis.decodedPicture.slice(" + strconv.Itoa(
			len(encoded),
		) + "," + strconv.Itoa(
			len(encoded)+48*1024,
		) + ")"
		piece, err := contract.Decode[string](f.eval(t, tab, expression))
		if err != nil || piece == "" {
			t.Fatalf("picture slice %v", err)
		}
		encoded += piece
	}
	f.mutate(t, tab, "delete globalThis.decodedPicture")
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		t.Fatal(err)
	}
	picture, err := png.Decode(bytes.NewReader(data))
	if err != nil {
		t.Fatal(err)
	}
	return picture
}

func TestNarrowStillPictureMatchesPageCoordinates(t *testing.T) {
	f := chromeFixture(t)
	path := filepath.Join(f.root, "still.html")
	if err := os.WriteFile(
		path,
		[]byte(
			`<!doctype html><style>body{margin:0;background:white}</style><div style="position:fixed;left:10px;top:100px;width:100px;height:30px;background:red"></div>`,
		),
		0o600,
	); err != nil {
		t.Fatal(err)
	}
	opened, err := browserproto.DecodeOpenResult(f.call(t, "open", browserArgs(t, `{"url":$0}`, "file://"+path)))
	if err != nil {
		t.Fatal(err)
	}
	tab := opened.Tab
	view := f.view(t)
	view.send(t, &browserproto.LiveViewerMessageHello{Platform: "mac"})
	view.until(t, "state", nil)
	view.send(t, &browserproto.LiveViewerMessageWatch{Tab: &tab})
	for _, size := range [][2]uint32{{409, 632}, {800, 600}, {409, 632}} {
		view.send(
			t,
			&browserproto.LiveViewerMessagePanel{
				Width:            size[0],
				Height:           size[1],
				DevicePixelRatio: 2,
				ScreenWidth:      1280,
				ScreenHeight:     720,
			},
		)
		stream := view.until(t, "stream", func(raw json.RawMessage) bool {
			return string(observedField(t, raw, "width")) == strconv.Itoa(int(size[0]*2)) &&
				string(observedField(t, raw, "height")) == strconv.Itoa(int(size[1]*2))
		})
		generation, err := contract.Decode[uint32](observedField(t, stream, "generation"))
		if err != nil {
			t.Fatal(err)
		}
		frame := view.picture(t, generation)
		if !frame.header.Key {
			t.Fatal("first picture is not a key frame")
		}
		picture := f.decodedPicture(t, tab, frame.video)
		for _, point := range [][2]int{{4, 4}, {int(size[0]*2) - 5, int(size[1]*2) - 5}} {
			r, g, b, _ := picture.At(point[0], point[1]).RGBA()
			if r>>8 <= 230 || g>>8 <= 230 || b>>8 <= 230 {
				t.Fatalf("white corner %v: %d %d %d", point, r>>8, g>>8, b>>8)
			}
		}
		r, g, b, _ := picture.At(40, 220).RGBA()
		if r>>8 <= 220 || g>>8 >= 35 || b>>8 >= 35 {
			t.Fatalf("red rectangle: %d %d %d", r>>8, g>>8, b>>8)
		}
	}
	view.close(t)
}

// stripeContrast measures the largest adjacent brightness difference in the fixture stripe row.
func stripeContrast(picture image.Image, row, from, to int) int {
	most := 0
	for x := from; x < to-1; x++ {
		a, _, _, _ := picture.At(x, row).RGBA()
		b, _, _, _ := picture.At(x+1, row).RGBA()
		difference := int(a>>8) - int(b>>8)
		if difference < 0 {
			difference = -difference
		}
		most = max(most, difference)
	}
	return most
}

func TestWatchedTabArrivesWithDetailOfViewersRatio(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	view := f.view(t)
	view.send(t, &browserproto.LiveViewerMessageHello{Platform: "linux"})
	view.send(
		t,
		&browserproto.LiveViewerMessagePanel{
			Width:            800,
			Height:           600,
			DevicePixelRatio: 1,
			ScreenWidth:      1440,
			ScreenHeight:     900,
		},
	)
	view.until(t, "state", nil)
	view.send(t, &browserproto.LiveViewerMessageWatch{Tab: &tab})
	contrast := []int{}
	for _, ratio := range []uint32{1, 2} {
		if ratio == 2 {
			view.send(
				t,
				&browserproto.LiveViewerMessagePanel{
					Width:            800,
					Height:           600,
					DevicePixelRatio: 2,
					ScreenWidth:      1440,
					ScreenHeight:     900,
				},
			)
		}
		stream := view.until(t, "stream", func(raw json.RawMessage) bool {
			return string(observedField(t, raw, "width")) == strconv.Itoa(int(800*ratio))
		})
		f.eventually(t, tab, "devicePixelRatio==="+strconv.Itoa(int(ratio)))
		generation, err := contract.Decode[uint32](observedField(t, stream, "generation"))
		if err != nil {
			t.Fatal(err)
		}
		frame := view.picture(t, generation)
		if !frame.header.Key || frame.header.Width != uint16(800*ratio) || ratio == 2 && frame.header.Height != 1200 {
			t.Fatalf("first picture %+v", frame.header)
		}
		picture := f.decodedPicture(t, tab, frame.video)
		if picture.Bounds().Dx() != int(800*ratio) {
			t.Fatal(picture.Bounds())
		}
		contrast = append(contrast, stripeContrast(picture, int(390*ratio), int(150*ratio), int(350*ratio)))
	}
	if contrast[0] >= 40 || contrast[1] <= 100 || contrast[1] <= contrast[0]*3 {
		t.Fatal(contrast)
	}
	shot := f.command(t, tab, "screenshot", `{"output":"shot.png"}`)
	expectValue(t, observedField(t, shot, "width"), 800)
	expectValue(t, observedField(t, shot, "height"), 600)
	ratio, err := contract.Decode[float64](observedField(t, shot, "viewport", "devicePixelRatio"))
	if err != nil || ratio != 2 {
		t.Fatalf("%s %v", shot, err)
	}
	view.close(t)
}

func TestCaptureExtensionReloadPreservesPagesAndRecreatesWorker(t *testing.T) {
	f := chromeFixture(t)
	opened, err := browserproto.DecodeOpenResult(f.call(t, "open", `{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	env := f.environment(t)
	endpoint, err := os.ReadFile(filepath.Join(filepath.Dir(env.DownloadDirectory()), "DevToolsActivePort"))
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Fields(string(endpoint))
	if len(lines) != 2 {
		t.Fatalf("endpoint %q", endpoint)
	}
	address := "ws://127.0.0.1:" + lines[0] + lines[1]
	workerURL := "chrome-extension://" + tabstest.CaptureExtensionID + "/background.js"
	previous := target.ID("")
	for round := 0; round < 4; round++ {
		// Each attempt is a real target listing; it repeats until the offscreen page and a new worker are listed.
		var worker target.ID
		for worker == "" {
			targets, err := tabstest.Targets(t.Context(), env)
			if err != nil {
				t.Fatal(err)
			}
			started := false
			for _, row := range targets {
				started = started || row.URL == strings.ReplaceAll(workerURL, "background.js", "offscreen.html")
			}
			if started {
				for _, row := range targets {
					if row.URL == workerURL && row.TargetID != previous {
						worker = row.TargetID
					}
				}
			}
		}
		if len(f.tabs(t)) != 1 {
			t.Fatal("extension listed as tab")
		}
		expectValue(t, f.eval(t, opened.Tab, "document.URL"), "about:blank")
		if round < 3 {
			value, err := cdptest.EvaluateIn(
				t.Context(),
				address,
				worker,
				"setTimeout(()=>chrome.runtime.reload(),100);true",
			)
			if err != nil {
				t.Fatal(err)
			}
			expectValue(t, value, true)
			previous = worker
		}
	}
}
