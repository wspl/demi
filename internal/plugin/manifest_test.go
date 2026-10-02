package plugin

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

// TestRustManifests protects the complete plugin registration wire; no IO
// beyond one fixture read, subprocesses, network or wall-clock waits.
func TestRustManifests(t *testing.T) {
	source, err := os.ReadFile("testdata/manifests.json")
	if err != nil {
		t.Fatal(err)
	}
	values, err := decodeManifests(source)
	if err != nil {
		t.Fatal(err)
	}
	wire, err := values.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	var formatted bytes.Buffer
	if err := json.Indent(&formatted, wire, "", "  "); err != nil {
		t.Fatal(err)
	}
	formatted.WriteByte('\n')
	if !bytes.Equal(formatted.Bytes(), source) {
		t.Fatalf("Rust manifest round trip differs\n%s", formatted.Bytes())
	}
}

// TestMinimalManifestDefaults checks absent Rust-defaulted fields at the plugin
// registration boundary. It uses only in-memory JSON, with no external resources.
func TestMinimalManifestDefaults(t *testing.T) {
	cases := []struct{ name, input, want string }{
		{"identity", `{"id":"test","name":"Test","description":"Test plugin"}`, `{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],"context":false,"streams":[]}`},
		{"page", `{"id":"test","name":"Test","description":"Test plugin","page":{"package":"@demicodes/test"}}`, `{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],"context":false,"streams":[],"page":{"package":"@demicodes/test","methods":[]}}`},
		{"contributions", `{"id":"test","name":"Test","description":"Test plugin","streams":[{"name":"live","operation":{"package":"test","operation":"live"},"receives":{},"sends":{}}],"page":{"package":"@demicodes/test","user":{"schema":{}},"conversation":{"schema":{}},"methods":[{"name":"read","scope":"user","params":{},"result":{}}]}}`, `{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],"context":false,"streams":[{"name":"live","operation":{"package":"test","operation":"live"},"receives":{},"sends":{},"constants":[]}],"page":{"package":"@demicodes/test","user":{"schema":{},"topics":[],"operations":[]},"conversation":{"schema":{},"topics":[],"operations":[]},"methods":[{"name":"read","scope":"user","params":{},"result":{},"operations":[]}]}}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			manifest, err := DecodeManifest([]byte(tc.input))
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := manifest.MarshalJSON()
			if err != nil {
				t.Fatal(err)
			}
			if string(encoded) != tc.want {
				t.Fatalf("manifest defaults: got %s; want %s", encoded, tc.want)
			}
		})
	}
}
