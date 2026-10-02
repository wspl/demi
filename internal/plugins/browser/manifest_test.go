package browser_test

import (
	"testing"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugins/browser"
)

func TestPageDeclarationsAndManifestOwnership(t *testing.T) {
	factory, err := browser.New()
	if err != nil {
		t.Fatal(err)
	}
	manifest := factory.Manifest()
	if manifest.ID != "browser" || manifest.Page.Package != "@demicodes/plugin-browser" {
		t.Fatalf("identity: %+v", manifest)
	}
	state := manifest.Page.Conversation
	if len(state.Topics) != 1 || state.Topics[0] != plugin.TopicJobs || state.Operations[0].Operation != "browser.tabs" {
		t.Fatalf("state: %+v", state)
	}
	for _, test := range []struct{ method, accepted, refused string }{
		{"open", `{}`, `{"url":""}`},
		{"close", `{"tab":"not-a-tab"}`, `{"tab":42}`},
		{"navigate", `{"tab":"t1","url":"https://example.test"}`, `{"tab":"t1","url":""}`},
		{"history", `{"tab":"t1","action":"back"}`, `{"tab":"t1","action":"home"}`},
	} {
		found := false
		for _, method := range manifest.Page.Methods {
			if method.Name != test.method {
				continue
			}
			found = true
			if method.Scope != plugin.ScopeConversation {
				t.Fatalf("scope: %s", method.Scope)
			}
			if err := method.Params.Check([]byte(test.accepted)); err != nil {
				t.Fatal(err)
			}
			if err := method.Params.Check([]byte(test.refused)); err == nil {
				t.Fatalf("%s accepted %s", test.method, test.refused)
			}
		}
		if !found {
			t.Fatalf("missing %s", test.method)
		}
	}
	manifest.Page.Methods[0].Operations[0].Operation = "changed"
	state.Topics[0] = plugin.TopicExposes
	state.Operations[0].Operation = "changed"
	manifest.Commands[0].Tree.Node.(*declare.Group[declare.NativeOperation]).Name = "changed"
	other := factory.Manifest()
	if other.Page.Methods[0].Operations[0].Operation != "browser.open" || other.Page.Conversation.Topics[0] != plugin.TopicJobs || other.Page.Conversation.Operations[0].Operation != "browser.tabs" || declare.Name(other.Commands[0].Tree.Node) != "browser" {
		t.Fatal("caller changed factory manifest")
	}
}

func TestLiveStreamBindingAndFrameConstants(t *testing.T) {
	// Message schemas must come from browserop; this tests only the plugin-owned binding and constants.
	stream, err := browser.LiveStream(nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if stream.Name != "browser" || stream.Operation.Package != "demi.browser" || stream.Operation.Operation != "browser.live" {
		t.Fatalf("binding: %+v", stream)
	}
	expected := map[string]string{
		"LIVE_CONTROL_FRAME": "1", "LIVE_VIDEO_FRAME": "2", "LIVE_FILE_FRAME": "3", "LIVE_MAX_FRAME_BYTES": "16777216", "LIVE_FILE_CHUNK_BYTES": "65536", "LIVE_VIDEO_HEADER_BYTES": "40", "LIVE_VIDEO_TAB_BYTES": "16", "LIVE_FILE_HEADER_BYTES": "8", "LIVE_HEARTBEAT_MS": "250", "LIVE_STALL_MS": "1000", "LIVE_VIDEO_CODEC": `"avc1.640033"`, "LIVE_CAPTURE_UNAVAILABLE": `"capture_unavailable"`, "LIVE_CAPTURE_FAILED": `"capture_failed"`,
	}
	if len(stream.Constants) != len(expected) {
		t.Fatalf("constants: %d", len(stream.Constants))
	}
	for _, constant := range stream.Constants {
		if string(constant.Value) != expected[constant.Name] || constant.Description == "" {
			t.Fatalf("constant: %+v", constant)
		}
	}
}
