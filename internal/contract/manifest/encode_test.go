package manifest

import "testing"

func TestEncodersRefuseInvalidValues(t *testing.T) {
	leaf := RPCLeaf{Name: "show", Summary: "Show a file"}
	if _, err := EncodeNodeJSON(Group{Name: "files", Summary: "Files", Subcommands: []Node{leaf}}); err != nil {
		t.Fatalf("a valid group: %v", err)
	}
	invalid := map[string]Node{
		"empty group":         Group{Name: "files", Summary: "Files", Subcommands: []Node{}},
		"missing subcommand":  Group{Name: "files", Summary: "Files", Subcommands: []Node{nil}},
		"invalid name":        RPCLeaf{Name: "-show", Summary: "Show"},
		"invalid nested name": Group{Name: "files", Summary: "Files", Subcommands: []Node{RPCLeaf{Name: "a b"}}},
	}
	for name, node := range invalid {
		if _, err := EncodeNodeJSON(node); err == nil {
			t.Errorf("%s: encoded", name)
		}
	}
}
