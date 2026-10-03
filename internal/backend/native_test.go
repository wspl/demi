package backend_test

import (
	"bytes"
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
	t.Parallel()
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
	conversationCreate(s.ctx, s.t, s.b, &s.user, hostsConversation)
	laptop := s.pair("laptop")
	s.move(laptop, laptop.Runner.Home())
	socket := conversationStream(s.ctx, t, b, &user, hostsConversation, "echo")
	conversationStreamReady(s.ctx, t, socket)
	if err := socket.Close(websocket.StatusNormalClosure, ""); err != nil {
		t.Fatal(err)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	artifact := built.Descriptor.Targets[string(target)]
	served := conversationRequest(s.ctx, s.t, s.b, nil, "GET", "/native-artifacts/"+artifact.SHA256, "", 200)
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
		conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, nil, "GET", "/native-artifacts/"+unknown, "", 404), webapi.ErrorCodeNotFound)
	}
}
