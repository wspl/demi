package browser

import (
	"math"
	"testing"
	"time"

	"github.com/wspl/demi/internal/contract/zodrt"
)

const tab = "t_abcdefghijklmnopqrstuv"

func TestCommands(t *testing.T) {
	command, err := ParseBrowserCommand("click", map[string]any{"tab": tab, "css": "#go", "wait-url": "**/done"})
	if err != nil {
		t.Fatal(err)
	}
	if CommandOperation(command) != "click" || CommandTimeout(command) != BrowserTimeoutMs*time.Millisecond {
		t.Errorf("click: %s %v", CommandOperation(command), CommandTimeout(command))
	}
	if url, ok := CommandWaitURL(command); !ok || url != "**/done" {
		t.Errorf("wait URL %q %v", url, ok)
	}
	if target, ok := CommandTarget(command); !ok || target.CSS.Value != "#go" {
		t.Errorf("target %#v", target)
	}
	open, err := ParseBrowserCommand("open", map[string]any{"url": "https://example.com"})
	if err != nil {
		t.Fatal(err)
	}
	if CommandTimeout(open) != BrowserMaxTimeoutMs*time.Millisecond {
		t.Errorf("open timeout %v", CommandTimeout(open))
	}
	if _, ok := CommandTab(open); ok {
		t.Error("open names a tab")
	}
	if _, err := ParseBrowserCommand("click", map[string]any{"tab": tab, "count": 3.0}); err == nil {
		t.Error("a triple click parsed")
	}
	if _, err := ParseBrowserCommand("teleport", map[string]any{}); err == nil {
		t.Error("an unknown operation parsed")
	}
	if _, err := ValidateResult("close", map[string]any{"closed": 1.0}); err == nil {
		t.Error("an invalid result validated")
	}
}

func TestEncodersRefuseInvalidValues(t *testing.T) {
	panel := LiveViewerPanel{Width: 800, Height: 600, DevicePixelRatio: 5, ScreenWidth: 800, ScreenHeight: 600}
	if _, err := EncodeLiveViewerMessageJSON(panel); err == nil {
		t.Error("a pixel ratio above 4 encoded")
	}
	panel.DevicePixelRatio = math.NaN()
	if _, err := EncodeLiveViewerMessageJSON(panel); err == nil {
		t.Error("NaN encoded")
	}
	query := BrowserQuery{Within: zodrt.Some[*BrowserQuery](nil)}
	if _, err := EncodeBrowserQueryJSON(query); err == nil {
		t.Error("a present nested query without a value encoded")
	}
	query.Within = zodrt.Some(&BrowserQuery{Nth: zodrt.Some(int64(-1))})
	if _, err := EncodeBrowserQueryJSON(query); err == nil {
		t.Error("a negative nested index encoded")
	}
}
