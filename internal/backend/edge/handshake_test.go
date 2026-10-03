package edge

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// The route has a real published stream and owned conversation. No shard is
// installed: reaching shard acquisition or Host admission fails the test rather
// than admitting anything. This test fails on the former Upgrade-only check.
// Cost: one temporary SQLite database, no processes, network, or model calls.
func TestBadUserStreamHandshakeNeverReachesAdmission(t *testing.T) {
	control, err := database.OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), core.SystemClock{})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := control.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	hash, err := database.ParsePasswordHash(
		"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo",
	)
	if err != nil {
		t.Fatal(err)
	}
	user, err := control.CreateMaster(t.Context(), "owner@example.test", hash)
	if err != nil {
		t.Fatal(user, err)
	}
	id := webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a00")
	if _, _, err := control.CreateConversation(t.Context(), user.ID, id); err != nil {
		t.Fatal(err)
	}
	native, err := runners.NewNativeCatalog(
		[]commandwire.PackageDescriptor{
			{
				ID:              "example.commands",
				Version:         "1",
				ProtocolVersion: 1,
				Operations:      []string{"fixture"},
				Targets:         map[string]commandwire.PackageArtifact{},
			},
		},
		&runners.UnpublishedStore{},
	)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := native.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	declarations := make([]hostaccess.StreamDeclaration, 1)
	declarations[0].Name = "fixture"
	declarations[0].Operation.Package = "example.commands"
	declarations[0].Operation.Operation = "fixture"
	streams := hostaccess.NewUserStreams(declarations, native)
	edge := &Edge{state: AppState{Services: &usershard.Services{Control: control, UserStreams: streams}}}
	for _, test := range []struct {
		name   string
		change func(*http.Request)
	}{
		{"method", func(r *http.Request) { r.Method = "HEAD" }},
		{"HTTP version", func(r *http.Request) { r.ProtoMinor = 0 }},
		{"missing Connection", func(r *http.Request) { r.Header.Del("Connection") }},
		{"wrong Connection token", func(r *http.Request) { r.Header.Set("Connection", "not-upgrade") }},
		{"missing Upgrade", func(r *http.Request) { r.Header.Del("Upgrade") }},
		{"wrong Upgrade token", func(r *http.Request) { r.Header.Set("Upgrade", "not-websocket") }},
		{"missing version", func(r *http.Request) { r.Header.Del("Sec-WebSocket-Version") }},
		{"wrong version", func(r *http.Request) { r.Header.Set("Sec-WebSocket-Version", "12") }},
		{"version list", func(r *http.Request) { r.Header.Set("Sec-WebSocket-Version", "13, 12") }},
		{"missing key", func(r *http.Request) { r.Header.Del("Sec-WebSocket-Key") }},
		{"empty key", func(r *http.Request) { r.Header.Set("Sec-WebSocket-Key", "") }},
		{"duplicate key", func(r *http.Request) { r.Header.Add("Sec-WebSocket-Key", r.Header.Get("Sec-WebSocket-Key")) }},
		{"invalid base64 key", func(r *http.Request) { r.Header.Set("Sec-WebSocket-Key", "!") }},
		{"short key", func(r *http.Request) { r.Header.Set("Sec-WebSocket-Key", "YQ==") }},
	} {
		t.Run(test.name, func(t *testing.T) {
			request := handshakeRequest()
			request.SetPathValue("id", string(id))
			request.SetPathValue("name", "fixture")
			request = request.WithContext(context.WithValue(t.Context(), userKey{}, user))
			test.change(request)
			answer := httptest.NewRecorder()
			serveEndpoint(edge.userStream)(answer, request)
			body, err := webapi.DecodeErrorBody(answer.Body.Bytes())
			if err != nil || answer.Code != 426 || body.Code != "upgrade_required" ||
				body.Message != "A user stream is a WebSocket" {
				t.Fatalf("%d %s: %v", answer.Code, answer.Body.String(), err)
			}
		})
	}
}

func handshakeRequest() *http.Request {
	r := httptest.NewRequest("GET", "/api/conversations/example/streams/fixture", nil)
	r.Header.Set("Connection", "Upgrade")
	r.Header.Set("Upgrade", "websocket")
	r.Header.Set("Sec-WebSocket-Version", "13")
	r.Header.Set("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
	return r
}

func TestPageHandshakeAcceptsLibraryTokenAndKeyForms(t *testing.T) {
	r := handshakeRequest()
	r.Header.Set("Connection", "keep-alive")
	r.Header.Add("Connection", "other, uPgRaDe ")
	r.Header.Set("Upgrade", "other")
	r.Header.Add("Upgrade", " WebSocket, another")
	r.Header.Set("Sec-WebSocket-Key", " \tdGhlIHNhbXBsZSBub25jZQ==\t ")
	if err := pageHandshake(r, "fixture"); err != nil {
		t.Fatal(err)
	}
}

func TestValidationRefusalsKeepStatusCodeAndField(t *testing.T) {
	for _, test := range []struct {
		name, code, field string
		decode            func() error
	}{
		{"body", "invalid_body", "email", func() error {
			_, err := decodeBody(
				httptest.NewRequest("POST", "/api/auth/login", strings.NewReader(`{"email":42,"password":"test"}`)),
				webapi.DecodeCredentials,
			)
			return err
		}},
		{"query", "invalid_query", "refresh", func() error {
			_, err := decodeQuery(httptest.NewRequest("GET", "/api/models?refresh=TRUE", nil), webapi.DecodeRefresh, "refresh")
			return err
		}},
	} {
		t.Run(test.name, func(t *testing.T) {
			err := test.decode()
			if err == nil {
				t.Fatal("invalid input accepted")
			}
			answer := httptest.NewRecorder()
			writeError(answer, err)
			body, failed := webapi.DecodeErrorBody(answer.Body.Bytes())
			if failed != nil || answer.Code != 400 || string(body.Code) != test.code ||
				!strings.Contains(body.Message, test.field) {
				t.Fatalf("%d %s: %v", answer.Code, answer.Body.String(), failed)
			}
		})
	}
}
