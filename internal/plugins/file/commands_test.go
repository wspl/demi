package file

import (
	"testing"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin/plugintest"
)

// Parse the arguments a runner consumes, including stdin bodies; below one second.
func TestNativeCommands(t *testing.T) {
	f, err := New()
	if err != nil {
		t.Fatal(err)
	}
	roots, err := plugintest.Roots(f.Manifest())
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		args              []string
		stdin             *string
		operation, values string
	}{
		{[]string{"file", "read", "a.txt"}, nil, "file.read", `{"path":"a.txt"}`},
		{[]string{"file", "create", "a.txt"}, new("<hello>&"), "file.create", `{"path":"a.txt","content":"<hello>&"}`},
		{
			[]string{
				"file",
				"edit",
				"a.txt",
				"--old",
				"old",
				"--new",
				"new",
			},
			nil,
			"file.edit",
			`{"path":"a.txt","old":"old","new":"new"}`,
		},
		{[]string{"file", "patch"}, new("patch"), "file.patch", `{"patch":"patch"}`},
	} {
		parsed, err := plugintest.Parse(roots[0], tc.args, tc.stdin)
		if err != nil {
			t.Fatal(err)
		}
		selected, err := roots[0].Select(tc.args)
		if err != nil {
			t.Fatal(err)
		}
		native, ok := selected.Node.(*declare.Leaf[declare.Binding]).Kind.(*declare.Native[declare.Binding])
		if !ok || native.Binding.Package != "demi.file" || native.Binding.Operation != tc.operation {
			t.Fatalf("wrong native binding: %+v", native)
		}
		values, err := parsed.Values.MarshalJSON()
		if err != nil {
			t.Fatal(err)
		}
		if string(values) != tc.values {
			t.Fatalf("%v: arguments %s, want %s", tc.args, values, tc.values)
		}
	}
}
