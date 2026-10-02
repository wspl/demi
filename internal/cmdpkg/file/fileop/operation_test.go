package fileop_test

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/file/fileop"
	"github.com/wspl/demi/internal/contract"
)

// These in-memory contract scenarios have no IO beyond reading schema fixtures
// and run within the ordinary one-second test budget.
func TestFileArgumentsRefuseEmptyOldTextAndZeroPositions(t *testing.T) {
	valid := `{"path":"a","old":"x","new":"y","occurrence":2}`
	op, err := fileop.Parse("file.edit", []byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	edit, ok := op.(*fileop.EditArgs)
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
			op, err := fileop.Parse(tc.operation, []byte(tc.args))
			var failure *fileop.OperationError
			if op != nil || !errors.As(err, &failure) || failure.Err == nil {
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
		want       fileop.Operation
	}{
		{"file.read", `{"path":"a"}`, &fileop.ReadArgs{Path: "a"}},
		{"file.create", `{"path":"a","content":""}`, &fileop.CreateArgs{Path: "a"}},
		{"file.edit", `{"path":"a","old":"x","new":""}`, &fileop.EditArgs{Path: "a", Old: "x"}},
		{"file.patch", `{"patch":""}`, &fileop.PatchArgs{}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := fileop.Parse(tc.name, []byte(tc.args))
			if err != nil || !reflect.DeepEqual(got, tc.want) {
				t.Fatalf("Parse = %#v, %v; want %#v", got, err, tc.want)
			}
		})
	}
	want := []string{"file.read", "file.create", "file.edit", "file.patch"}
	if !reflect.DeepEqual(fileop.Operations(), want) {
		t.Fatalf("Operations = %v; want %v", fileop.Operations(), want)
	}
	for _, name := range fileop.Operations() {
		_, err := fileop.Parse(name, []byte(`{}`))
		var failure *fileop.OperationError
		if !errors.As(err, &failure) || failure.Err == nil {
			t.Fatalf("listed %s was not decoded: %v", name, err)
		}
	}
	for _, name := range []string{"file.remove", "browser.tabs"} {
		_, err := fileop.Parse(name, []byte(`{}`))
		var failure *fileop.OperationError
		if !errors.As(err, &failure) || failure.Err != nil || failure.Name != name {
			t.Fatalf("unknown %s: %v", name, err)
		}
	}
}

func TestSchemasMatchRustManifest(t *testing.T) {
	for _, tc := range []struct {
		name   string
		schema func() json.RawMessage
	}{
		{"read", fileop.ReadArgsJSONSchema},
		{"create", fileop.CreateArgsJSONSchema},
		{"edit", fileop.EditArgsJSONSchema},
		{"patch", fileop.PatchArgsJSONSchema},
	} {
		t.Run(tc.name, func(t *testing.T) {
			data, err := os.ReadFile("testdata/" + tc.name + ".json")
			if err != nil {
				t.Fatal(err)
			}
			// Schemas are test metadata, not incoming operation arguments.
			var got, want any
			if err := json.Unmarshal(data, &want); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(tc.schema(), &got); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("schema differs from Rust manifest\ngot: %s\nwant: %s", tc.schema(), data)
			}
		})
	}
}
