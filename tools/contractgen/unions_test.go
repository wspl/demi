package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/unions"
)

// These wire scenarios use local fixtures only; the budget is one second.
func TestBooleanReplies(t *testing.T) {
	for _, doc := range []string{
		`{"ok":true,"platform":"darwin-arm64","installed":[{"version":"2.1.3","path":"/opt/claude"}]}`,
		`{"ok":false,"code":"install_failed","message":"digest differs"}`,
	} {
		value, err := unions.DecodeStatusReply([]byte(doc))
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := json.Marshal(unions.StatusReplyJSON{Value: value})
		if err != nil {
			t.Fatal(err)
		}
		if string(encoded) != doc {
			t.Fatalf("reply changed: %s", encoded)
		}
		packed, err := unions.EncodeStatusReplyMsgpack(value)
		if err != nil {
			t.Fatal(err)
		}
		restored, err := unions.DecodeStatusReplyMsgpack(packed)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(value, restored) {
			t.Fatalf("MessagePack reply changed: %#v", restored)
		}
	}
	for _, doc := range []string{
		`{"ok":true,"version":"2.1.3","path":"/opt/claude"}`,
		`{"ok":false,"code":"invalid_release","message":"bad version"}`,
		`{"ok":false,"code":"unsupported_platform","message":"no binary"}`,
	} {
		value, err := unions.DecodeEnsureReply([]byte(doc))
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := json.Marshal(unions.EnsureReplyJSON{Value: value})
		if err != nil {
			t.Fatal(err)
		}
		if string(encoded) != doc {
			t.Fatalf("reply changed: %s", encoded)
		}
	}
	for _, doc := range []string{
		`{"ok":"true","platform":"x","installed":[]}`,
		`{"platform":"x","installed":[]}`,
		`{"ok":true,"platform":"x","installed":[],"code":"install_failed"}`,
		`{"ok":false,"code":"unknown","message":"x"}`,
		`{"ok":null,"platform":"x","installed":[]}`,
	} {
		if _, err := unions.DecodeStatusReply([]byte(doc)); err == nil {
			t.Fatalf("accepted %s", doc)
		}

		var malformed map[string]any
		if err := json.Unmarshal([]byte(doc), &malformed); err != nil {
			t.Fatal(err)
		}
		packed, err := contract.EncodeMsgpack(malformed)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := unions.DecodeStatusReplyMsgpack(packed); err == nil {
			t.Fatalf("accepted MessagePack form of %s", doc)
		}
	}
}

func TestUntaggedRunnerCorpus(t *testing.T) {
	paths, err := filepath.Glob("testdata/unions/fixtures/*.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) == 0 {
		t.Fatal("missing corpus")
	}
	for _, path := range paths {
		t.Run(filepath.Base(path), func(t *testing.T) {
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			value, err := unions.DecodeFrameMsgpack(data)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := unions.EncodeFrameMsgpack(value)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(data, encoded) {
				t.Fatalf("runner bytes changed:\n%x\n%x", data, encoded)
			}
			doc, err := json.Marshal(unions.FrameJSON{Value: value})
			if err != nil {
				t.Fatal(err)
			}
			restored, err := unions.DecodeFrame(doc)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(value, restored) {
				t.Fatalf("JSON changed runner value: %s", doc)
			}
		})
	}
	for _, doc := range []string{
		`{}`,
		`{"path":""}`,
		`{"path":"/x","url":"https://x.test"}`,
		`{"url":null}`,
		`{"path":"/x","unknown":true}`,
	} {
		if _, err := unions.DecodeArtifactLocation([]byte(doc)); err == nil {
			t.Fatalf("accepted %s", doc)
		}
	}
	for _, doc := range []string{`{}`, `{"jobId":"j","manifestHash":"invalid"}`, `{"streamId":"s","jobId":"j"}`} {
		if _, err := unions.DecodeArtifactOwner([]byte(doc)); err == nil {
			t.Fatalf("accepted %s", doc)
		}
	}
}

func TestUntaggedDeclarationOrder(t *testing.T) {
	value, err := unions.DecodeOrdered([]byte(`{"value":"first"}`))
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := value.(*unions.ZFirst); !ok {
		t.Fatalf("selected %T", value)
	}
	data, err := unions.EncodeOrderedMsgpack(value)
	if err != nil {
		t.Fatal(err)
	}
	value, err = unions.DecodeOrderedMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := value.(*unions.ZFirst); !ok {
		t.Fatalf("selected %T", value)
	}
}

func TestUntaggedManifestTrees(t *testing.T) {
	for _, name := range []string{"cli.json", "manifest.json"} {
		t.Run(name, func(t *testing.T) {
			data, err := os.ReadFile(filepath.Join("testdata/unions/fixtures", name))
			if err != nil {
				t.Fatal(err)
			}
			// The fixture envelope is test data; the tree itself enters its generated decoder.
			var document map[string]json.RawMessage
			if err := json.Unmarshal(data, &document); err != nil {
				t.Fatal(err)
			}
			if raw, ok := document["manifest"]; ok {
				if err := json.Unmarshal(raw, &document); err != nil {
					t.Fatal(err)
				}
			}
			var roots map[string]struct {
				Tree json.RawMessage `json:"tree"`
			}
			if err := json.Unmarshal(document["roots"], &roots); err != nil {
				t.Fatal(err)
			}
			if len(roots) == 0 {
				t.Fatal("fixture has no trees")
			}
			for _, root := range roots {
				value, err := unions.DecodeNode(root.Tree)
				if err != nil {
					t.Fatal(err)
				}
				encoded, err := json.Marshal(unions.NodeJSON{Value: value})
				if err != nil {
					t.Fatal(err)
				}
				var compact bytes.Buffer
				if err := json.Compact(&compact, root.Tree); err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(compact.Bytes(), encoded) {
					t.Fatalf("tree bytes changed:\n%s\n%s", compact.Bytes(), encoded)
				}
			}
		})
	}
	for _, doc := range []string{
		`{"name":"x","summary":"x"}`,
		`{"name":"x","summary":"x","subcommands":[],"kind":"rpc"}`,
		`{"name":"x","summary":"x","subcommands":null}`,
	} {
		if _, err := unions.DecodeNode([]byte(doc)); err == nil {
			t.Fatalf("accepted %s", doc)
		}
	}
}

func TestAbsentCollections(t *testing.T) {
	for _, doc := range []string{`{}`, `{"resources":{},"items":[],"bytes":""}`} {
		value, err := unions.DecodeEmpty([]byte(doc))
		if err != nil {
			t.Fatal(err)
		}
		if value.Resources == nil || value.Items == nil || value.Bytes == nil {
			t.Fatal("absent collection is not empty")
		}
		encoded, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if string(encoded) != "{}" {
			t.Fatalf("empty fields written: %s", encoded)
		}
		packed, err := value.MarshalMsgpack()
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(packed, []byte{0x80}) {
			t.Fatalf("empty fields written: %x", packed)
		}
		restored, err := unions.DecodeEmptyMsgpack(packed)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(value, restored) {
			t.Fatalf("absent MessagePack collections differ: %#v", restored)
		}
	}
	full := unions.Empty{Resources: map[string]string{"a": "b"}, Items: []string{"x"}, Bytes: unions.Bytes{0xff, 0x00}}
	for _, value := range []unions.Empty{full, {}} {
		data, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		restored, err := unions.DecodeEmpty(data)
		if err != nil {
			t.Fatal(err)
		}
		if len(value.Bytes) > 0 && !reflect.DeepEqual(value, restored) {
			t.Fatalf("nonempty JSON collections changed: %#v", restored)
		}
		packed, err := value.MarshalMsgpack()
		if err != nil {
			t.Fatal(err)
		}
		restored, err = unions.DecodeEmptyMsgpack(packed)
		if err != nil {
			t.Fatal(err)
		}
		if len(value.Bytes) > 0 && !reflect.DeepEqual(value, restored) {
			t.Fatalf("nonempty MessagePack collections changed: %#v", restored)
		}
	}
	for _, doc := range []string{`{"resources":null}`, `{"items":null}`, `{"bytes":null}`, `{"bytes":"/wA"}`} {
		if _, err := unions.DecodeEmpty([]byte(doc)); err == nil {
			t.Fatalf("accepted %s", doc)
		}
	}
	data, err := contract.EncodeMsgpackObject(
		[]contract.Field{
			{Name: "resources", Value: map[string]string{}},
			{Name: "items", Value: []string{}},
			{Name: "bytes", Value: []byte{}},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	value, err := unions.DecodeEmptyMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := value.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(encoded, []byte{0x80}) {
		t.Fatalf("present empty fields retained: %x", encoded)
	}
	for _, field := range []string{"resources", "items", "bytes"} {
		data, err := contract.EncodeMsgpackObject([]contract.Field{{Name: field, Value: nil}})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := unions.DecodeEmptyMsgpack(data); err == nil {
			t.Fatalf("accepted null %s", field)
		}
	}
	var nilItems []string
	leaf := unions.Leaf{Name: "x", Summary: "x", Kind: "rpc", Positionals: &nilItems}
	if _, err := json.Marshal(leaf); err == nil {
		t.Fatal("pointer to nil collection must not encode null")
	}
}

// One loader invocation protects discovery, TypeScript tags and public exports;
// its budget is one second and it starts no processes beyond the Go loader.
func TestUnionGeneration(t *testing.T) {
	if err := generate(t.Context(), []string{"./testdata/unions"}, false, "", true); err != nil {
		t.Fatal(err)
	}
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/unions", "./testdata/tables"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dest, "plugin-unions/plugin.ts"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"z.literal(true)",
		"z.literal(false)",
		"const failureSchema",
		"const bytesSchema",
		"export const wireApiSchema",
		"export type WireApi",
		"export const blockIdSchema",
		"export type BlockId",
		"export const httpFailureRecordSchema",
		"export type HttpFailureRecord",
	} {
		if !strings.Contains(string(data), want) {
			t.Errorf("missing %s", want)
		}
	}
	for _, bad := range []string{
		"export const failureSchema",
		"export const bytesSchema",
		"compiledSchema",
		"z.literal(\"true\")",
	} {
		if strings.Contains(string(data), bad) {
			t.Errorf("unexpected %s", bad)
		}
	}
	tables, err := os.ReadFile(filepath.Join(dest, "protocol/tables.ts"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"export const PREVIEW_TYPES",
		"export const ATTACHMENT_FILE_EXTENSIONS: readonly string[]",
		"export const VIDEO_FILE_EXTENSIONS: readonly string[]",
	} {
		if !strings.Contains(string(tables), want) {
			t.Errorf("missing %s", want)
		}
	}
}

// Union accessors remain callable after decoding, and never become wire fields.
// This exercises both generated and hand-written seals; budget <1 second, no IO.
func TestUnionExportedMethods(t *testing.T) {
	for _, tc := range []struct {
		tag      string
		editable bool
	}{
		{"editable", true},
		{"fixed", false},
	} {
		t.Run(tc.tag, func(t *testing.T) {
			wire := `{"type":"` + tc.tag + `","text":"document_id"}`
			value, err := unions.DecodeDocument([]byte(wire))
			if err != nil {
				t.Fatal(err)
			}
			if value.ID() != "document_id" || value.CreatedAt() != "2026-10-03T00:00:00.000Z" ||
				value.Model() != "test" ||
				value.IsEditable() != tc.editable {
				t.Fatalf("decoded union lost its accessors: %#v", value)
			}
			encoded, err := (unions.DocumentJSON{Value: value}).MarshalJSON()
			if err != nil {
				t.Fatal(err)
			}
			if string(encoded) != wire {
				t.Fatalf("encoded %s; want %s", encoded, wire)
			}
		})
	}
}
