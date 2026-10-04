package browser_test

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugins/browser"
)

// TestManifestCallersCannotChangeFactory checks that Manifest returns
// independently owned declarations: a caller that edits its copy leaves the
// next caller's manifest unchanged. Memory only, well under one second.
func TestManifestCallersCannotChangeFactory(t *testing.T) {
	factory, err := browser.New()
	if err != nil {
		t.Fatal(err)
	}
	manifest := factory.Manifest()
	manifest.Streams[0].Constants[0].Value[0] = '9'
	manifest.Streams[0].Constants[0].Name = "changed"
	manifest.Page.Methods[0].Operations[0].Operation = "changed"
	manifest.Page.Conversation.Topics[0] = plugin.TopicExposes
	manifest.Page.Conversation.Operations[0].Operation = "changed"
	manifest.Commands[0].Tree.Node.(*declare.Group[declare.NativeOperation]).Name = "changed"
	other := factory.Manifest()
	if string(other.Streams[0].Constants[0].Value) != "1" ||
		other.Streams[0].Constants[0].Name != "LIVE_CONTROL_FRAME" {
		t.Fatal("caller changed factory stream constants")
	}
	if other.Page.Methods[0].Operations[0].Operation != "browser.open" ||
		other.Page.Conversation.Topics[0] != plugin.TopicJobs ||
		other.Page.Conversation.Operations[0].Operation != "browser.tabs" ||
		declare.Name(other.Commands[0].Tree.Node) != "browser" {
		t.Fatal("caller changed factory manifest")
	}
}

// TestManifestMatchesFixture compares the factory's complete wire declaration with the
// fixture testdata/manifest.json, including schema member order. It uses only memory
// and one local fixture read.
func TestManifestMatchesFixture(t *testing.T) {
	factory, err := browser.New()
	if err != nil {
		t.Fatal(err)
	}
	got, err := factory.Manifest().MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	source, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	var want bytes.Buffer
	if err := json.Compact(&want, source); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want.Bytes()) {
		offset := 0
		for offset < min(len(got), want.Len()) && got[offset] == want.Bytes()[offset] {
			offset++
		}
		t.Fatalf(
			"manifest differs from fixture (%d bytes, want %d); first difference at %d",
			len(got),
			want.Len(),
			offset,
		)
	}
}
