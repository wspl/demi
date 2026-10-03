package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/features"
	"github.com/wspl/demi/tools/contractgen/testdata/text"
	"github.com/wspl/demi/tools/contractgen/testdata/unions"
)

// Generated methods preserve escaping through scalars, union holders, nested
// objects, and opaque input. In-memory only; budget <1 second.
func TestGeneratedJSONEscaping(t *testing.T) {
	const value = "<>&\u2028\u2029x"
	for _, tc := range []struct {
		name  string
		value json.Marshaler
		want  string
	}{
		{"scalar", text.Name(value), `"` + value + `"`},
		{"object", features.Flat{Common: features.Common{ID: "id_a"}, Text: value}, `{"id":"id_a","text":"` + value + `"}`},
		{
			"union",
			unions.StatusReplyJSON{
				Value: &unions.Status{
					Platform: value,
					Installed: []unions.Installed{
						{
							Version: value,
							Path:    value,
						},
					},
				},
			},
			`{"ok":true,"platform":"` + value + `","installed":[{"version":"` + value + `","path":"` + value + `"}]}`,
		},
		{
			"opaque",
			&unions.Leaf{
				Kind:    "rpc",
				Name:    value,
				Summary: value,
				Input:   new(json.RawMessage(`{"\u003c":"\u2028"}`)),
			},
			`{"name":"` + value + `","summary":"` + value + `","input":{"<":"` + "\u2028" + `"},"kind":"rpc"}`,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := tc.value.MarshalJSON()
			if err != nil || string(got) != tc.want {
				t.Fatalf("MarshalJSON = %s, %v; want %s", got, err, tc.want)
			}
		})
	}
}

// Tolerant contracts check ignored fields too. In-memory only; budget <1 second.
func TestGeneratedJSONRecursionLimit(t *testing.T) {
	for _, depth := range []int{126, 127} {
		data := `{"name":"id_a","children":[],"unknown":` + strings.Repeat(
			"[",
			depth,
		) + "0" + strings.Repeat(
			"]",
			depth,
		) + "}"
		_, err := features.DecodeTree([]byte(data))
		if (err == nil) != (depth == 126) {
			t.Fatalf("%d containers including root: %v", depth+1, err)
		}
	}
}
