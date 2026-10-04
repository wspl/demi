package plugin_test

import (
	"testing"

	"github.com/wspl/demi/internal/plugin"
)

// TestMinimalManifestDefaults checks that a manifest's absent defaulted fields
// encode as empty lists and false at the plugin registration boundary. It uses
// only in-memory JSON, with no external resources.
func TestMinimalManifestDefaults(t *testing.T) {
	cases := []struct{ name, input, want string }{
		{
			"identity",
			`{"id":"test","name":"Test","description":"Test plugin"}`,
			`{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],"context":false,"streams":[]}`,
		},
		{
			"page",
			`{"id":"test","name":"Test","description":"Test plugin","page":{"package":"@demicodes/test"}}`,
			`{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],` +
				`"context":false,"streams":[],"page":{"package":"@demicodes/test","methods":[]}}`,
		},
		{
			"contributions",
			`{"id":"test","name":"Test","description":"Test plugin","streams":[{"name":"live",` +
				`"operation":{"package":"test","operation":"live"},"receives":{},"sends":{}}],` +
				`"page":{"package":"@demicodes/test","user":{"schema":{}},` +
				`"conversation":{"schema":{}},"methods":[{"name":"read","scope":"user","params":{},` +
				`"result":{}}]}}`,
			`{"id":"test","name":"Test","description":"Test plugin","commands":[],"profiles":[],` +
				`"context":false,"streams":[{"name":"live","operation":{"package":"test",` +
				`"operation":"live"},"receives":{},"sends":{},"constants":[]}],` +
				`"page":{"package":"@demicodes/test","user":{"schema":{},"topics":[],"operations":[]},` +
				`"conversation":{"schema":{},"topics":[],"operations":[]},"methods":[{"name":"read",` +
				`"scope":"user","params":{},"result":{},"operations":[]}]}}`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			manifest, err := plugin.DecodeManifest([]byte(tc.input))
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
