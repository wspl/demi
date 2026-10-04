//go:build acceptance

package live

import (
	"context"
	"encoding/binary"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

type chromeViewerBrowser struct {
	environment       *tabs.Environment
	hub               *Hub
	released, changed chan struct{}
}

func (b *chromeViewerBrowser) Running(ctx context.Context) (*tabs.Environment, *Hub, error) {
	return b.environment, b.hub, ctx.Err()
}

func (b *chromeViewerBrowser) Changed() <-chan struct{} {
	return b.changed
}

func (b *chromeViewerBrowser) Released() <-chan struct{} {
	return b.released
}

// TestChromeLiveAcceptance costs one Chrome launch and capture startup. A real
// browser is required to prove that the extension supplies video and viewer key
// events reach page JavaScript; all pages and recorded events stay on loopback.
func TestChromeLiveAcceptance(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to the pinned Chrome for Testing executable")
	}
	// cancel ends Serve before the deferred join below.
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	keys := make(chan string, 4)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/key" {
			select {
			case keys <- r.URL.Query().Get("value"):
			case <-r.Context().Done():
			}
			w.WriteHeader(http.StatusNoContent)
			return
		}
		w.Header().Set("Content-Type", "text/html")
		if _, err := fmt.Fprint(w, `<!doctype html><input autofocus><script>
 document.addEventListener('keydown', e => fetch('/key?value='+encodeURIComponent(e.key)));
 let hue=0; function paint(){document.body.style.background='hsl('+(hue++%360)+' 50% 60%)';`+
			`requestAnimationFrame(paint)} paint();
 </script>`); err != nil {
			t.Log(err)
		}
	}))
	defer server.Close()
	environment := tabstest.Launch(
		ctx,
		t,
		tabs.LaunchOptions{
			Executable: executable,
			Locale:     cmdproto.CommandLocale{TimeZone: "UTC", Languages: []cmdproto.LanguageTag{"en-US"}},
		},
	)
	hub, err := Start(environment)
	if err != nil {
		t.Fatal(err)
	}
	tab, err := environment.Open(ctx, server.URL, 30*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	source := &viewerSource{data: make(chan []byte, 8)}
	output, records := cmdsdk.OutputChannel(ctx)
	browser := &chromeViewerBrowser{
		environment: environment,
		hub:         hub,
		released:    make(chan struct{}),
		changed:     make(chan struct{}),
	}
	completed := make(chan error, 1)
	go func() {
		completion, err := Serve(
			ctx,
			browser,
			cmdsdk.InvocationContext[cmdproto.Invocation]{Input: cmdsdk.NewInput(source), Output: output},
		)
		if err == nil && completion.ExitCode != 0 {
			err = fmt.Errorf("live completion: %+v", completion)
		}
		completed <- err
	}()
	defer func() {
		cancel()
		<-completed
	}()
	send := func(message browserproto.LiveViewerMessage) {
		t.Helper()
		data, err := (browserproto.LiveViewerMessageJSON{Value: message}).MarshalJSON()
		if err != nil {
			t.Fatal(err)
		}
		select {
		case source.data <- framed(browserproto.ControlFrame, data):
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	id := tab.ID()
	send(&browserproto.LiveViewerMessageHello{Platform: "mac"})
	send(
		&browserproto.LiveViewerMessagePanel{
			Width:            640,
			Height:           480,
			DevicePixelRatio: 1,
			ScreenWidth:      1280,
			ScreenHeight:     720,
		},
	)
	send(&browserproto.LiveViewerMessageWatch{Tab: &id})
	var pending []byte
	video := false
	for !video {
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case record := <-records:
			data, ok := record.(cmdproto.Stdout)
			if !ok {
				continue
			}
			pending = append(pending, data...)
			for len(pending) >= 4 {
				length := int(binary.BigEndian.Uint32(pending))
				if length == 0 || length > browserproto.MaxFrameBytes {
					t.Fatalf("bad frame length %d", length)
				}
				if len(pending) < 4+length {
					break
				}
				frame := pending[4 : 4+length]
				pending = pending[4+length:]
				switch frame[0] {
				case browserproto.ControlFrame:
					message, err := browserproto.DecodeLiveModuleMessage(frame[1:])
					if err != nil {
						t.Fatal(err)
					}
					if notice, ok := message.(*browserproto.LiveModuleMessageNotice); ok {
						t.Fatalf("live notice: %s: %s", notice.Code, notice.Message)
					}
				case browserproto.VideoFrame:
					header, payload, err := browserproto.SplitVideoFrame(frame[1:])
					if err != nil {
						t.Fatal(err)
					}
					if header.Tab != id || !header.Key || len(payload) == 0 || header.Width == 0 || header.Height == 0 {
						t.Fatalf("first video: %+v, %d bytes", header, len(payload))
					}
					send(&browserproto.LiveViewerMessageAck{Generation: header.Generation, Sequence: header.Sequence})
					video = true
				}
				if video {
					break
				}
			}
		}
	}
	send(&browserproto.LiveViewerMessageKey{Tab: id, Action: "down", Key: "a", Code: "KeyA", KeyCode: 65})
	send(&browserproto.LiveViewerMessageKey{Tab: id, Action: "up", Key: "a", Code: "KeyA", KeyCode: 65})
	// Continue draining output so delivery backpressure cannot block input.
	for {
		select {
		case key := <-keys:
			if key != "a" {
				t.Fatalf("page key=%q", key)
			}
			return
		case <-records:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
}
