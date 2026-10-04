package fileproto_test

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/file/fileproto"
	"github.com/wspl/demi/internal/contract"
)

// These in-memory contract scenarios have no IO beyond reading schema fixtures
// and run within the ordinary one-second test budget.
func TestFileArgumentsRefuseEmptyOldTextAndZeroPositions(t *testing.T) {
	valid := `{"path":"a","old":"x","new":"y","occurrence":2}`
	op, err := fileproto.Parse("file.edit", []byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	edit, ok := op.(*fileproto.EditArgs)
	if !ok || edit.Occurrence == nil || *edit.Occurrence != 2 {
		t.Fatalf("decoded edit = %#v", op)
	}
	for _, tc := range []struct{ name, operation, args string }{
		{"empty old", "file.edit", `{"path":"a","old":"","new":"y"}`},
		{"zero occurrence", "file.edit", `{"path":"a","old":"x","new":"y","occurrence":0}`},
		{"zero context", "file.edit", `{"path":"a","old":"x","new":"y","context":0}`},
		{"null context", "file.edit", `{"path":"a","old":"x","new":"y","context":null}`},
		{"missing new", "file.edit", `{"path":"a","old":"x"}`},
		{"unknown field", "file.read", `{"path":"a","extra":true}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			op, err := fileproto.Parse(tc.operation, []byte(tc.args))
			if op != nil || err == nil || errors.Is(err, fileproto.ErrUnknownOperation) {
				t.Fatalf("Parse = %#v, %v; want invalid arguments", op, err)
			}
			var field *contract.Error
			if !errors.As(err, &field) {
				t.Fatalf("argument error lost its field cause: %v", err)
			}
		})
	}
}

func TestListedOperationsAreTheOnesThePackageDecodes(t *testing.T) {
	for _, tc := range []struct {
		name, args string
		want       fileproto.Operation
	}{
		{"file.read", `{"path":"a"}`, &fileproto.ReadArgs{Path: "a"}},
		{"file.create", `{"path":"a","content":""}`, &fileproto.CreateArgs{Path: "a"}},
		{"file.edit", `{"path":"a","old":"x","new":""}`, &fileproto.EditArgs{Path: "a", Old: "x"}},
		{"file.patch", `{"patch":""}`, &fileproto.PatchArgs{}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := fileproto.Parse(tc.name, []byte(tc.args))
			if err != nil || !reflect.DeepEqual(got, tc.want) {
				t.Fatalf("Parse = %#v, %v; want %#v", got, err, tc.want)
			}
		})
	}
	want := []string{"file.read", "file.create", "file.edit", "file.patch"}
	if !reflect.DeepEqual(fileproto.Operations(), want) {
		t.Fatalf("Operations = %v; want %v", fileproto.Operations(), want)
	}
	for _, name := range fileproto.Operations() {
		_, err := fileproto.Parse(name, []byte(`{}`))
		if err == nil || errors.Is(err, fileproto.ErrUnknownOperation) {
			t.Fatalf("listed %s was not decoded: %v", name, err)
		}
	}
	for _, name := range []string{"file.remove", "browser.tabs"} {
		_, err := fileproto.Parse(name, []byte(`{}`))
		if !errors.Is(err, fileproto.ErrUnknownOperation) || err.Error() != "unknown operation "+name {
			t.Fatalf("unknown %s: %v", name, err)
		}
	}
}

func TestSchemasMatchManifestFixtures(t *testing.T) {
	for _, tc := range []struct {
		name   string
		schema func() json.RawMessage
	}{
		{"read", fileproto.ReadArgsJSONSchema},
		{"create", fileproto.CreateArgsJSONSchema},
		{"edit", fileproto.EditArgsJSONSchema},
		{"patch", fileproto.PatchArgsJSONSchema},
	} {
		t.Run(tc.name, func(t *testing.T) {
			data, err := os.ReadFile("testdata/" + tc.name + ".json")
			if err != nil {
				t.Fatal(err)
			}
			var want bytes.Buffer
			if err := json.Compact(&want, data); err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(tc.schema(), want.Bytes()) {
				t.Fatalf("schema differs from the manifest fixture\ngot: %s\nwant: %s", tc.schema(), data)
			}
		})
	}
}
