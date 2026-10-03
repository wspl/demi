//go:build acceptance

package backend_test

import (
	"bytes"
	"context"
	"encoding/binary"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider/providertest"
)

// realView frames the browser user stream, whose socket chunks have no message boundaries.
type realView struct {
	socket  *websocket.Conn
	pending []byte
}

func (v *realView) send(ctx context.Context, t *testing.T, message browserop.LiveViewerMessage) {
	t.Helper()
	data, err := contract.EncodeJSON(message)
	wireMust(t, err)
	frame := binary.BigEndian.AppendUint32(nil, uint32(1+len(data)))
	frame = append(frame, browserop.ControlFrame)
	wireMust(t, v.socket.Write(ctx, websocket.MessageBinary, append(frame, data...)))
}

func (v *realView) next(ctx context.Context, t *testing.T) (browserop.LiveModuleMessage, *browserop.VideoHeader, []byte) {
	t.Helper()
	for {
		if len(v.pending) >= 4 {
			length := int(binary.BigEndian.Uint32(v.pending[:4]))
			if len(v.pending) >= 4+length {
				frame := v.pending[4 : 4+length]
				v.pending = v.pending[4+length:]
				if len(frame) == 0 {
					t.Fatal("empty live frame")
				}
				switch frame[0] {
				case browserop.ControlFrame:
					message, err := browserop.DecodeLiveModuleMessage(frame[1:])
					wireMust(t, err)
					return message, nil, nil
				case browserop.VideoFrame:
					header, data, err := browserop.SplitVideoFrame(frame[1:])
					wireMust(t, err)
					return nil, &header, data
				default:
					t.Fatalf("unknown live frame kind %d", frame[0])
				}
			}
		}
		readCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
		kind, data, err := v.socket.Read(readCtx)
		cancel()
		wireMust(t, err)
		if kind == websocket.MessageBinary {
			v.pending = append(v.pending, data...)
		}
	}
}

// About 13 seconds: a real runner installs demi.browser, launches Chrome and
// encodes the first live picture. No model request leaves the fixture vendor.
func TestAnAgentDrivesChromeOnAPairedDeviceWhichTheUserWatchesUntilRelease(t *testing.T) {
	chrome := os.Getenv("DEMI_TEST_CHROME")
	if chrome == "" {
		t.Skip("the browser suite: needs DEMI_TEST_CHROME and an ordinary user (scenarios.md § Browser suite)")
	}
	const id = "7b6a5c4d-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	const greeting = "Hello from the fixture page"
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	page := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html")
		// A browser that went away needs no answer.
		_, _ = fmt.Fprintf(w, `<!doctype html><title>Fixture</title><h1>%s</h1><div style="width: 40px; height: 40px; background: red"></div>`, greeting)
	}))
	defer page.Close()
	h, manager, err := backendtest.HostsHarness(ctx, t)
	wireMust(t, err)
	built, err := backendtest.BuildPackage(ctx, t, "demi-browser")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	s := &hostScenario{t, ctx, h, b, master, manager}
	laptop := s.pair("laptop")
	pinned, err := browserop.PinnedRelease()
	wireMust(t, err)
	target, err := commandwire.HostTarget()
	wireMust(t, err)
	platform := pinned.Platform(string(target))
	if platform == nil {
		t.Fatal("the pinned release has no machine target")
	}
	cache := filepath.Join(laptop.Runner.StateDir(), "artifacts")
	browsers := filepath.Join(cache, platform.SHA256)
	_, err = artifactstest.InstallUnpacked(ctx, cache, artifacts.Archive{Digest: artifacts.Digest{Size: platform.Size, SHA256: platform.SHA256}, Entry: platform.Executable}, chrome)
	wireMust(t, err)
	vendor := providertest.StartVendor(t)
	provider := conversationAnthropic(ctx, t, b, &master, vendor)
	conversationCreate(ctx, t, b, &master, id)
	conversationRequest(ctx, t, b, &master, "PATCH", "/api/conversations/"+id, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, laptop.ID(), laptop.Runner.Home()), 200)
	work := &cloudWork{s: s, vendor: vendor, provider: provider, id: id}
	work.open()
	output := work.turn("browse", "demi browser open "+page.URL+" && demi browser content read t1 --format text", "read", 120000)
	for _, want := range []string{"exitCode: 0", "Tab: t1", greeting} {
		if !strings.Contains(output, want) {
			t.Fatalf("missing %q: %s", want, output)
		}
	}
	processes, err := backendtest.ChromeProcesses(ctx, browsers)
	wireMust(t, err)
	if len(processes) == 0 {
		t.Fatal("the device runs no conversation Chrome")
	}
	view := &realView{socket: conversationStream(ctx, t, b, &master, id, "browser")}
	view.send(ctx, t, &browserop.LiveViewerMessageHello{Platform: "linux"})
	view.send(ctx, t, &browserop.LiveViewerMessagePanel{Width: 800, Height: 600, DevicePixelRatio: 1, ScreenWidth: 1440, ScreenHeight: 900})
	for {
		message, _, _ := view.next(ctx, t)
		if state, ok := message.(*browserop.LiveModuleMessageState); ok {
			if len(state.Tabs) == 0 || state.Tabs[0].ID != "t1" {
				t.Fatalf("state: %+v", state)
			}
			break
		}
	}
	tab := browserop.TabID("t1")
	view.send(ctx, t, &browserop.LiveViewerMessageWatch{Tab: &tab})
	var generation uint32
	for {
		message, _, _ := view.next(ctx, t)
		if stream, ok := message.(*browserop.LiveModuleMessageStream); ok {
			conversationEqual(t, stream.Tab, tab)
			generation = stream.Generation
			break
		}
	}
	for {
		message, header, data := view.next(ctx, t)
		if stream, ok := message.(*browserop.LiveModuleMessageStream); ok {
			generation = stream.Generation
		}
		if header == nil {
			continue
		}
		view.send(ctx, t, &browserop.LiveViewerMessageAck{Generation: header.Generation, Sequence: header.Sequence})
		if header.Generation != generation {
			continue
		}
		conversationEqual(t, header.Tab, tab)
		if !header.Key {
			t.Fatal("a stream must start with a key frame")
		}
		if !bytes.HasPrefix(data, []byte{0, 0, 0, 1}) {
			t.Fatal("not H.264 Annex B")
		}
		break
	}
	wireMust(t, view.socket.CloseNow())
	conversationRequest(ctx, t, b, &master, "PATCH", "/api/conversations/"+id, `{"archived":true}`, 200)
	for {
		processes, err := backendtest.ChromeProcesses(ctx, browsers)
		wireMust(t, err)
		if len(processes) == 0 {
			break
		}
		wireMust(t, ctx.Err())
	}
	wireMust(t, laptop.Runner.Stop(ctx))
	wireMust(t, b.Close(ctx))
}
