package main

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/tools/contractgen/pagemeta"
)

type factory struct{ manifest plugin.Manifest }

func (f factory) Manifest() plugin.Manifest { return f.manifest }
func (factory) Instance() plugin.Plugin     { return nil }

// Cost: the checked-in Rust browser manifest, no backend or processes.
// Observe the command's JSON interface, including registration order and every use.
func TestPrintedPageManifests(t *testing.T) {
	data, err := os.ReadFile("../testdata/pluginbrowser/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := plugin.DecodeManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	manifest.Page.User = manifest.Page.Conversation
	manifest.Streams[0].Constants = append(manifest.Streams[0].Constants, plugin.Constant{
		Name: "OPAQUE", Description: "An ordered JSON value.",
		Value: json.RawMessage(`{"z":"<>&  ","a":null}`),
	})
	empty := plugin.Manifest{ID: "changes", Page: &plugin.Page{Package: "@demicodes/plugin-changes"}}
	factories := []plugin.Factory{
		factory{plugin.Manifest{ID: "without-page"}},
		factory{manifest},
		factory{empty},
	}
	var output bytes.Buffer
	if err := writePages(&output, factories); err != nil {
		t.Fatal(err)
	}
	pages, err := pagemeta.Decode(output.Bytes())
	if err != nil {
		t.Fatal(err)
	}
	if len(pages) != 2 || pages[0].ID != "browser" || pages[1].ID != "changes" {
		t.Fatalf("printer changed registration order or page selection: %s", &output)
	}
	want := []pagemeta.Schema{
		{Direction: "receive", Value: manifest.Page.User.Schema.Document()},
		{Direction: "receive", Value: manifest.Page.Conversation.Schema.Document()},
	}
	for _, method := range manifest.Page.Methods {
		want = append(want,
			pagemeta.Schema{Direction: "send", Value: method.Params.Document()},
			pagemeta.Schema{Direction: "receive", Value: method.Result.Document()})
	}
	want = append(want,
		pagemeta.Schema{Direction: "receive", Value: manifest.Streams[0].Receives.Document()},
		pagemeta.Schema{Direction: "send", Value: manifest.Streams[0].Sends.Document()})
	compactJSON := cmp.Transformer("compactJSON", func(raw json.RawMessage) string {
		var compact bytes.Buffer
		if err := json.Compact(&compact, raw); err != nil {
			t.Fatal(err)
		}
		return compact.String()
	})
	if diff := cmp.Diff(want, pages[0].Schemas, compactJSON); diff != "" {
		t.Fatalf("page use directions (-want +got):\n%s", diff)
	}
	if pages[0].Package != manifest.Page.Package || len(pages[0].Constants) != len(manifest.Streams[0].Constants) {
		t.Fatalf("page identity or constants lost: %+v", pages[0])
	}
	for i, constant := range manifest.Streams[0].Constants {
		got := pages[0].Constants[i]
		var gotValue, wantValue any
		if err := json.Unmarshal(got.Value, &gotValue); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(constant.Value, &wantValue); err != nil {
			t.Fatal(err)
		}
		if got.Name != constant.Name || got.Description != constant.Description || cmp.Diff(gotValue, wantValue) != "" {
			t.Errorf("stream constant differs: %+v / %+v", got, constant)
		}
	}
}
