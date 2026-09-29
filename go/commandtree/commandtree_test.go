package commandtree

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

// fixtureTree reads the tree of the recorded CLI fixture, which the runner's
// tests share (crates/command-tree/tests/fixtures/cli.json).
func fixtureTree(t *testing.T) Node {
	t.Helper()
	data, err := os.ReadFile("testdata/cli.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Manifest struct {
			Roots map[string]struct {
				Tree jsontext.Value `json:"tree"`
			} `json:"roots"`
		} `json:"manifest"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	node, err := DecodeNode(fixture.Manifest.Roots["fixture"].Tree)
	if err != nil {
		t.Fatal(err)
	}
	return node
}

func TestHelpAndCommandLinesMatchTheRecordedCases(t *testing.T) {
	data, err := os.ReadFile("testdata/cli.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Help  string `json:"help"`
		Cases []struct {
			Argv    []string       `json:"argv"`
			Stdin   *string        `json:"stdin"`
			Invalid bool           `json:"invalid"`
			Parsed  jsontext.Value `json:"parsed"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	tree := fixtureTree(t)
	if err := Validate(tree); err != nil {
		t.Fatal(err)
	}
	if got := Help(tree, "fixture"); got != fixture.Help {
		t.Errorf("help\n got:\n%s\nwant:\n%s", got, fixture.Help)
	}
	for _, test := range fixture.Cases {
		parsed, err := read(tree, test.Argv, test.Stdin)
		if test.Invalid {
			if err == nil {
				t.Errorf("argv=%q is accepted", test.Argv)
			}
			continue
		}
		if err != nil {
			t.Errorf("argv=%q: %v", test.Argv, err)
			continue
		}
		want := mustDecodeValue(t, test.Parsed)
		if got := parsedValue(t, parsed); !sameJSON(t, got, want) {
			t.Errorf("argv=%q\n got %s\nwant %s", test.Argv, encodeValue(got), test.Parsed)
		}
	}
}

// sameJSON reports whether two values are the same JSON, whatever the order of
// the members of their objects.
func sameJSON(t *testing.T, a, b any) bool {
	t.Helper()
	var x, y any
	if err := json.Unmarshal([]byte(encodeValue(a)), &x); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(encodeValue(b)), &y); err != nil {
		t.Fatal(err)
	}
	return reflect.DeepEqual(x, y)
}

func TestValidatingLeavesTheParsedValuesAsTheyWere(t *testing.T) {
	tree := fixtureTree(t)
	argv := []string{"read", "p", "--count", "3"}
	selected, err := Select(tree, argv)
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := selected.Parse(argv)
	if err != nil {
		t.Fatal(err)
	}
	body := "b"
	validated, err := parsed.Validate(selected.Node.(Leaf), &body)
	if err != nil {
		t.Fatal(err)
	}
	if got, _ := parsed.Values.Get("count"); got != "3" {
		t.Errorf("the parsed count = %#v, want the text 3", got)
	}
	if _, ok := parsed.Values.Get("body"); ok {
		t.Error("the parsed values gained the stdin body")
	}
	if got, _ := validated.Values.Get("count"); got != int64(3) {
		t.Errorf("the validated count = %#v, want the number 3", got)
	}
}

// declarations builds a tree of a group with an rpc leaf and two native leaves,
// as the backend declares one: a native leaf names its package and operation
// and has no descriptor hash yet.
func declarations() Node {
	native := func(name, pkg, operation string) Node {
		return Leaf{Name: name, Summary: "S", Kind: KindNative, Binding: &Binding{Package: pkg, Operation: operation}}
	}
	return Group{Name: "demi", Summary: "Demi", Subcommands: []Node{
		Leaf{Name: "todo", Summary: "S", Kind: KindRPC},
		Group{Name: "file", Summary: "Files", Subcommands: []Node{
			native("read", "demi.builtin", "file.read"),
			native("create", "demi.builtin", "file.create"),
		}},
	}}
}

func TestPinningPinsEachNativeCommandToItsDescriptor(t *testing.T) {
	tree := declarations()
	var asked []NativeOperation
	pinned, err := Pin(tree, func(operation NativeOperation) (string, error) {
		asked = append(asked, operation)
		return "hash-of-" + operation.Operation, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	wantAsked := []NativeOperation{
		{Package: "demi.builtin", Operation: "file.read"},
		{Package: "demi.builtin", Operation: "file.create"},
	}
	if !reflect.DeepEqual(asked, wantAsked) {
		t.Errorf("asked for %v, want %v", asked, wantAsked)
	}
	var hashes []string
	for _, leaf := range Leaves(pinned) {
		if leaf.Binding != nil {
			hashes = append(hashes, leaf.Binding.DescriptorHash)
		}
	}
	if want := []string{"hash-of-file.read", "hash-of-file.create"}; !reflect.DeepEqual(hashes, want) {
		t.Errorf("pinned hashes = %v, want %v", hashes, want)
	}
	for _, leaf := range Leaves(tree) {
		if leaf.Binding != nil && leaf.Binding.DescriptorHash != "" {
			t.Errorf("the declaration's %s was pinned: %+v", leaf.Name, leaf.Binding)
		}
	}
	if got := Name(pinned); got != "demi" {
		t.Errorf("name = %q", got)
	}
	if got := Summary(pinned); got != "Demi" {
		t.Errorf("summary = %q", got)
	}
}

func TestPinningStopsAtTheFirstDescriptorThatIsMissing(t *testing.T) {
	missing := errors.New("no descriptor")
	calls := 0
	_, err := Pin(declarations(), func(NativeOperation) (string, error) {
		calls++
		return "", missing
	})
	if !errors.Is(err, missing) || calls != 1 {
		t.Errorf("err = %v after %d calls, want the descriptor's error after 1", err, calls)
	}
}

func TestANativeLeafWithoutABindingCannotBePinned(t *testing.T) {
	_, err := Pin(Leaf{Name: "read", Summary: "S", Kind: KindNative}, func(NativeOperation) (string, error) {
		return "", nil
	})
	var declaration *DeclarationError
	if !errors.As(err, &declaration) {
		t.Errorf("err = %v, want a *DeclarationError", err)
	}
}

func TestAnObjectKeepsItsMembersInOrder(t *testing.T) {
	value, err := DecodeValue([]byte(`{"b": 1, "a": {"y": 1.50, "x": [true, null]}, "c": "text"}`))
	if err != nil {
		t.Fatal(err)
	}
	object := value.(Object)
	var names []string
	for name := range object.All() {
		names = append(names, name)
	}
	if want := []string{"b", "a", "c"}; !reflect.DeepEqual(names, want) {
		t.Errorf("names = %v, want %v", names, want)
	}
	object.Set("b", "again")
	object.Set("d", true)
	data, err := json.Marshal(object)
	if err != nil {
		t.Fatal(err)
	}
	// A replaced member keeps its place, and a number keeps the text it had.
	if want := `{"b":"again","a":{"y":1.50,"x":[true,null]},"c":"text","d":true}`; string(data) != want {
		t.Errorf("encoded %s, want %s", data, want)
	}
	if _, err := DecodeValue([]byte(`{"a": 1, "a": 2}`)); err == nil {
		t.Error("a duplicate member is accepted")
	}
	if _, err := DecodeValue([]byte(`{"a": 1} x`)); err == nil {
		t.Error("text after the value is accepted")
	}
}

func TestASchemaIsCompiledOnceAndCheckedByEveryCaller(t *testing.T) {
	var schema Schema
	document := `{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"],"additionalProperties":false}`
	if err := json.Unmarshal([]byte(document), &schema); err != nil {
		t.Fatal(err)
	}
	fits, breaks := mustDecodeValue(t, []byte(`{"n": 1}`)), mustDecodeValue(t, []byte(`{"n": "x"}`))
	var group sync.WaitGroup
	for range 8 {
		group.Go(func() {
			for range 50 {
				if err := schema.Check(fits); err != nil {
					t.Error(err)
				}
				err := schema.Check(breaks)
				if err == nil || err.Error() != `"n" is not of type "integer"` {
					t.Errorf("err = %v", err)
				}
			}
		})
	}
	group.Wait()
}

func TestASchemaNeverReadsAnotherDocument(t *testing.T) {
	// The documents the schemas refer to exist and are valid schemas, so only
	// the refusal to load them keeps them out.
	dir := t.TempDir()
	path := filepath.Join(dir, "other.json")
	if err := os.WriteFile(path, []byte(`{"type": "integer"}`), 0o644); err != nil {
		t.Fatal(err)
	}
	var requests atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		_, _ = w.Write([]byte(`{"type": "integer"}`))
	}))
	defer server.Close()
	references := []string{
		"file://" + filepath.ToSlash(path),
		server.URL + "/other.json",
		"other.json",
		"#/$defs/missing",
	}
	for _, reference := range references {
		var schema Schema
		document := `{"$ref": "` + reference + `"}`
		if err := json.Unmarshal([]byte(document), &schema); err == nil {
			t.Errorf("%s: a schema that refers to another document is accepted", reference)
		}
	}
	if n := requests.Load(); n != 0 {
		t.Errorf("the server saw %d requests, want none", n)
	}
	// A reference inside the document is the schema's own.
	var schema Schema
	document := `{"$defs": {"n": {"type": "integer"}}, "properties": {"a": {"$ref": "#/$defs/n"}}}`
	if err := json.Unmarshal([]byte(document), &schema); err != nil {
		t.Errorf("a reference inside the document is refused: %v", err)
	}
}

func TestADocumentThatIsNotANodeIsRefusedNamingWhy(t *testing.T) {
	for name, test := range map[string]struct{ document, want string }{
		"an unknown kind":               {`{"name":"a","summary":"S","kind":"other"}`, "invalid: kind: must be one of: rpc, native"},
		"an rpc leaf with a binding":    {`{"name":"a","summary":"S","kind":"rpc","binding":{"package":"p","operation":"o","descriptorHash":"h"}}`, "invalid: an rpc command has no binding"},
		"a native leaf without one":     {`{"name":"a","summary":"S","kind":"native"}`, "invalid: a native command names its binding"},
		"a node that is not an object":  {`3`, "invalid: must be an object"},
		"a node that is neither":        {`{"name":"a","summary":"S"}`, "invalid: matches none of: Group, Leaf"},
		"a leaf with an unknown member": {`{"name":"a","summary":"S","kind":"rpc","extra":1}`, "invalid: matches none of: Group, Leaf"},
		"a duplicate member":            {`{"name":"a","summary":"S","kind":"rpc","input":{"type":"object","type":"object"}}`, "invalid: input.type: has a duplicate member"},
		"text after the node":           {`{"name":"a","summary":"S","kind":"rpc"} x`, "invalid: is not valid JSON"},
	} {
		_, err := DecodeNode([]byte(test.document))
		if err == nil || err.Error() != test.want {
			t.Errorf("%s: error = %v, want %q", name, err, test.want)
		}
	}
}

func TestALeafExplainsWhyItsMemberIsRefused(t *testing.T) {
	for name, test := range map[string]struct{ document, want string }{
		"an unknown member":             {`{"name":"a","summary":"S","kind":"rpc","extra":1}`, "invalid: extra: unknown member"},
		"a null option":                 {`{"name":"a","summary":"S","kind":"rpc","successOutput":null}`, "invalid: successOutput: must not be null"},
		"a missing kind":                {`{"name":"a","summary":"S"}`, "invalid: kind: required"},
		"an input of the wrong kind":    {`{"name":"a","summary":"S","kind":"rpc","input":[]}`, "invalid: input: must be an object"},
		"a positional that is not text": {`{"name":"a","summary":"S","kind":"rpc","positionals":[1]}`, "invalid: positionals[0]: must be a string"},
		"a schema that is not one":      {`{"name":"a","summary":"S","kind":"rpc","input":{"type":"nope"}}`, "invalid: input: is not a valid JSON Schema: "},
	} {
		var leaf Leaf
		err := json.Unmarshal([]byte(test.document), &leaf, wireOptions)
		var invalid *InvalidError
		if !errors.As(err, &invalid) || !strings.HasPrefix(invalid.Error(), test.want) {
			t.Errorf("%s: error = %v, want %q", name, err, test.want)
		}
	}
}

func TestANodeThatBreaksItsRulesIsNotEncoded(t *testing.T) {
	_, err := EncodeNode(Leaf{Name: "a", Summary: "S", Kind: KindRPC, Binding: &Binding{}})
	if err == nil || err.Error() != "invalid: an rpc command has no binding" {
		t.Errorf("err = %v", err)
	}
}

func TestAnInstanceThatIsNotJSONIsRefusedAsACallersMistake(t *testing.T) {
	var schema Schema
	if err := json.Unmarshal([]byte(`{"type":"object"}`), &schema); err != nil {
		t.Fatal(err)
	}
	err := schema.Check(make(chan int))
	if err == nil || !strings.HasPrefix(err.Error(), "commandtree: cannot check the instance") {
		t.Errorf("err = %v", err)
	}
}

func TestASchemaWordsOneFailureInOneOrderWhetherDeclaredOrDecoded(t *testing.T) {
	var declared Object
	for name, value := range map[string]string{
		"type": `"object"`, "required": `["a","b"]`, "additionalProperties": `false`,
		"properties": `{"a":{"type":"integer"}}`,
	} {
		member, err := DecodeValue([]byte(value))
		if err != nil {
			t.Fatal(err)
		}
		declared.Set(name, member)
	}
	schema, err := NewSchema(declared)
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(schema)
	if err != nil {
		t.Fatal(err)
	}
	var decoded Schema
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}
	instance := mustDecodeValue(t, []byte(`{"z": 1}`))
	want := `Additional properties are not allowed ('z' was unexpected); "a" is a required property; "b" is a required property`
	for name, s := range map[string]*Schema{"declared": schema, "decoded": &decoded} {
		if err := s.Check(instance); err == nil || err.Error() != want {
			t.Errorf("%s: %v, want %s", name, err, want)
		}
	}
}
