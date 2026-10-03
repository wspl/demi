package backend_test

import (
	"bytes"
	"net/http"
	"os"
	"strings"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/webapi"
)

// A real native stream installs the built fixture through the backend's development store.
func TestNativeDevelopmentReleaseServesOnlyLoadedArtifacts(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-native-fixture")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UseNativeFixture(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	s.create(hostsConversation)
	laptop := s.pair("laptop")
	s.move(laptop, laptop.Runner.Home())
	socket, response, err := websocket.Dial(s.ctx, b.WSURL("/api/conversations/"+hostsConversation+"/streams/echo"), &websocket.DialOptions{HTTPHeader: http.Header{"Cookie": {user.Cookie}, "Origin": {b.URL}}})
	if response != nil && response.Body != nil {
		_ = response.Body.Close()
	}
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = socket.CloseNow() }() // Socket reads own disconnection errors.
	payload := []byte("ping")
	if err := socket.Write(s.ctx, websocket.MessageBinary, payload); err != nil {
		t.Fatal(err)
	}
	var received []byte
	for len(received) < len(payload) {
		kind, data, err := socket.Read(s.ctx)
		if err != nil {
			t.Fatal(err)
		}
		if kind != websocket.MessageBinary {
			t.Fatal(kind)
		}
		received = append(received, data...)
	}
	if !bytes.Equal(received, payload) {
		t.Fatal("echo changed bytes")
	}
	if err := socket.Close(websocket.StatusNormalClosure, ""); err != nil {
		t.Fatal(err)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	artifact := built.Descriptor.Targets[string(target)]
	served := s.request("GET", "/native-artifacts/"+artifact.SHA256, "", 200)
	if served.Headers.Get("Cache-Control") != "public, max-age=31536000, immutable" {
		t.Fatal(served.Headers)
	}
	if served.Headers.Get("Content-Encoding") != "zstd" {
		t.Fatal(served.Headers)
	}
	if uint64(len(served.Body)) >= artifact.Size {
		t.Fatal("artifact not compressed")
	}
	client := artifacts.NewClientAllowingHTTP()
	defer client.Close()
	var decoded bytes.Buffer
	if err := artifacts.Download(s.ctx, client, b.URL+"/native-artifacts/"+artifact.SHA256, artifacts.Digest{Size: artifact.Size, SHA256: artifact.SHA256}, &decoded); err != nil {
		t.Fatal(err)
	}
	program, err := os.ReadFile(built.Program)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded.Bytes(), program) {
		t.Fatal("served executable differs")
	}
	for _, unknown := range []string{strings.Repeat("0", 64), "demi-native-fixture"} {
		s.refusal("GET", "/native-artifacts/"+unknown, "", 404, webapi.ErrorCodeNotFound)
	}
}
