package page

import (
	"bytes"
	"encoding/json"
	"testing"

	"github.com/chromedp/cdproto/accessibility"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/contract"
)

// Accessibility values of unsupported types are omitted without losing their nodes; under 1 s.
func TestAXValuesOmitUnsupportedTypesWithoutLosingNodes(t *testing.T) {
	for _, tc := range []struct {
		kind        string
		value, want json.RawMessage
	}{
		{"boolean", json.RawMessage(`false`), nil},
		{"valueUndefined", json.RawMessage(`null`), nil},
		{"string", json.RawMessage(`"input value"`), json.RawMessage(`"input value"`)},
		{"number", json.RawMessage(`42`), json.RawMessage(`42.0`)},
	} {
		t.Run(tc.kind, func(t *testing.T) {
			observation := observation{
				nodes: []observedNode{
					{
						ax: &accessibility.Node{
							NodeID: "1",
							Role:   &accessibility.Value{Type: "role", Value: json.RawMessage(`"textbox"`)},
							Value:  &accessibility.Value{Type: accessibility.ValueType(tc.kind), Value: tc.value},
							Properties: []*accessibility.Property{
								{Name: "labelledby", Value: &accessibility.Value{Type: "idrefList"}},
								{
									Name:  "focusable",
									Value: &accessibility.Value{Type: "boolean", Value: json.RawMessage(`true`)},
								},
							},
						},
						frame:  "frame",
						loader: "loader",
					},
				},
				order: []nodeDepth{{0, 0}},
			}
			nodes, truncated, err := observation.tree(&tabs.References{}, 100)
			if err != nil {
				t.Fatal(err)
			}
			if truncated || len(nodes) != 1 || nodes[0].Role != "textbox" {
				t.Fatalf("nodes=%+v truncated=%v", nodes, truncated)
			}
			if len(nodes[0].States) != 1 || nodes[0].States[0] != "focusable=true" {
				t.Fatalf("states=%v, want only focusable=true", nodes[0].States)
			}
			if tc.want == nil {
				encoded, err := contract.EncodeJSON(nodes[0])
				if err != nil || bytes.Contains(encoded, []byte(`"value":`)) {
					t.Fatalf("unsupported value was serialized: %s %v", encoded, err)
				}
				if nodes[0].Value != nil {
					t.Fatalf("unexpected value: %v", nodes[0].Value)
				}
				return
			}
			if nodes[0].Value == nil {
				t.Fatal("missing scalar")
			}
			encoded, err := (browserop.NodeValueJSON{Value: *nodes[0].Value}).MarshalJSON()
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			if err = json.Unmarshal(encoded, &got); err != nil {
				t.Fatal(err)
			}
			if err = json.Unmarshal(tc.want, &want); err != nil {
				t.Fatal(err)
			}
			if got != want {
				t.Fatalf("value=%v, want %v", got, want)
			}
		})
	}
}
