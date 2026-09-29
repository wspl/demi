package commandtree

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// The tables in testdata/rust record what the Rust implementation
// (crates/command-tree) answers for each case, the words of every message a
// user or the model reads. Each case here runs the Go implementation on the
// same input and expects the same answer. A case whose Go answer differs only in
// the set or the order of the failures records the Go answer in want, the Rust's
// in rust, and says why in reason. The tables are rewritten by running
// their cases through the Rust crate.

func readTable(t *testing.T, name string, table any) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("testdata", "rust", name))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, table); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
}

// decodeSchema reads a schema document as a declaration does.
func decodeSchema(t *testing.T, document jsontext.Value) (*Schema, error) {
	t.Helper()
	var schema Schema
	if err := json.Unmarshal(document, &schema); err != nil {
		return nil, err
	}
	return &schema, nil
}

func mustDecodeValue(t *testing.T, data []byte) any {
	t.Helper()
	value, err := DecodeValue(data)
	if err != nil {
		t.Fatalf("%s: %v", data, err)
	}
	return value
}

func TestFailuresAreWordedAndOrderedAsTheRustWordsThem(t *testing.T) {
	var table []struct {
		Name     string         `json:"name"`
		Schema   jsontext.Value `json:"schema"`
		Instance jsontext.Value `json:"instance"`
		Want     string         `json:"want"`
	}
	readTable(t, "wording.json", &table)
	for _, test := range table {
		t.Run(test.Name, func(t *testing.T) {
			schema, err := decodeSchema(t, test.Schema)
			if err != nil {
				t.Fatal(err)
			}
			got := ""
			if err := schema.Check(mustDecodeValue(t, test.Instance)); err != nil {
				got = err.Error()
			}
			if got != test.Want {
				t.Errorf("schema %s, instance %s\n got: %s\nwant: %s", test.Schema, test.Instance, got, test.Want)
			}
		})
	}
}

func TestASchemaThatIsNotOneIsRefusedWhenItIsRead(t *testing.T) {
	var table []struct {
		Name    string         `json:"name"`
		Schema  jsontext.Value `json:"schema"`
		Refused bool           `json:"refused"`
	}
	readTable(t, "invalid_schemas.json", &table)
	for _, test := range table {
		_, err := decodeSchema(t, test.Schema)
		if (err != nil) != test.Refused {
			t.Errorf("%s: error = %v, the Rust refuses: %v", test.Name, err, test.Refused)
		}
	}
}

func TestTheInputSubsetRefusesWhatTheRustRefusesInItsWords(t *testing.T) {
	var table []struct {
		Name        string         `json:"name"`
		Schema      jsontext.Value `json:"schema"`
		Want        string         `json:"want"`
		Declaration jsontext.Value `json:"declaration"`
	}
	readTable(t, "subset.json", &table)
	for _, test := range table {
		t.Run(test.Name, func(t *testing.T) {
			schema, err := decodeSchema(t, test.Schema)
			if test.Declaration != nil {
				// A document that is not a schema never reaches the subset.
				if err == nil {
					t.Fatalf("the Rust refuses the schema as invalid (%s), and the Go accepts it", test.Declaration)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			got := ""
			if err := CheckInputSubset(schema); err != nil {
				var declaration *DeclarationError
				if !errors.As(err, &declaration) {
					t.Fatalf("the refusal is a %T, not a *DeclarationError", err)
				}
				got = err.Error()
			}
			if got != test.Want {
				t.Errorf("schema %s\n got: %q\nwant: %q", test.Schema, got, test.Want)
			}
		})
	}
}

func TestATreeBreaksTheRulesOfDeclarationsAsTheRustSays(t *testing.T) {
	var table []struct {
		Name   string         `json:"name"`
		Tree   jsontext.Value `json:"tree"`
		Want   string         `json:"want"`
		Decode jsontext.Value `json:"decode"`
	}
	readTable(t, "declarations.json", &table)
	for _, test := range table {
		t.Run(test.Name, func(t *testing.T) {
			node, err := DecodeNode(test.Tree)
			if test.Decode != nil {
				if err == nil {
					t.Fatalf("the Rust refuses the node as it decodes it (%s), and the Go accepts it", test.Decode)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			got := ""
			if err := Validate(node); err != nil {
				var declaration *DeclarationError
				if !errors.As(err, &declaration) {
					t.Fatalf("the refusal is a %T, not a *DeclarationError", err)
				}
				got = err.Error()
			}
			if got != test.Want {
				t.Errorf("got %q, want %q", got, test.Want)
			}
		})
	}
}

func TestANodeIsDecodedAndEncodedAsTheRustDoes(t *testing.T) {
	var table []struct {
		Name     string         `json:"name"`
		Node     jsontext.Value `json:"node"`
		Accepted bool           `json:"accepted"`
		Encoded  jsontext.Value `json:"encoded"`
	}
	readTable(t, "decoding.json", &table)
	for _, test := range table {
		t.Run(test.Name, func(t *testing.T) {
			node, err := DecodeNode(test.Node)
			if !test.Accepted {
				if err == nil {
					t.Fatal("the Rust refuses the node, and the Go accepts it")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := EncodeNode(node)
			if err != nil {
				t.Fatal(err)
			}
			// The members of each object are compared in order too.
			got, want := mustDecodeValue(t, encoded), mustDecodeValue(t, test.Encoded)
			if !reflect.DeepEqual(got, want) {
				t.Errorf("encoded\n got: %s\nwant: %s", encoded, test.Encoded)
			}
		})
	}
}

// A recorded command line and what it comes to.
type lineCase struct {
	Tree  string         `json:"tree"`
	Argv  []string       `json:"argv"`
	Stdin *string        `json:"stdin"`
	Error string         `json:"error"`
	Value jsontext.Value `json:"parsed"`
}

// read is what a command line comes to as the runner reads it: argv after the
// root's name, and the body read from stdin for a leaf that takes one.
func read(root Node, argv []string, stdin *string) (Parsed, error) {
	selected, err := Select(root, argv)
	if err != nil {
		return Parsed{}, err
	}
	parsed, err := selected.Parse(argv)
	if err != nil {
		return Parsed{}, err
	}
	if leaf, ok := selected.Node.(Leaf); ok && !parsed.Help {
		return parsed.Validate(leaf, stdin)
	}
	return parsed, nil
}

// parsedValue returns what a [Parsed] is as the values of JSON, with its
// numbers as float64 so that 2 and 2.0 are the same number.
func parsedValue(t *testing.T, parsed Parsed) any {
	t.Helper()
	var values Object
	values.Set("path", stringsToValues(parsed.Path))
	values.Set("values", parsed.Values)
	values.Set("json", parsed.JSON)
	values.Set("help", parsed.Help)
	data, err := json.Marshal(values)
	if err != nil {
		t.Fatal(err)
	}
	return numbersAsFloats(mustDecodeValue(t, data))
}

func stringsToValues(texts []string) []any {
	values := make([]any, len(texts))
	for i, text := range texts {
		values[i] = text
	}
	return values
}

// numbersAsFloats returns value with each number as a float64.
func numbersAsFloats(value any) any {
	switch value := value.(type) {
	case Object:
		var converted Object
		for name, member := range value.All() {
			converted.Set(name, numbersAsFloats(member))
		}
		return converted
	case []any:
		converted := make([]any, len(value))
		for i, element := range value {
			converted[i] = numbersAsFloats(element)
		}
		return converted
	case interface{ Float64() (float64, error) }:
		number, _ := value.Float64()
		return number
	}
	return value
}

func TestCommandLinesAreReadAsTheRustReadsThem(t *testing.T) {
	var table struct {
		Trees map[string]jsontext.Value `json:"trees"`
		Cases []lineCase                `json:"cases"`
	}
	readTable(t, "lines.json", &table)
	trees := map[string]Node{}
	for name, raw := range table.Trees {
		node, err := DecodeNode(raw)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if err := Validate(node); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		trees[name] = node
	}
	for _, test := range table.Cases {
		t.Run(test.Tree+" "+strings.Join(test.Argv, " "), func(t *testing.T) {
			parsed, err := read(trees[test.Tree], test.Argv, test.Stdin)
			if test.Error != "" {
				if err == nil {
					t.Fatalf("accepted, the Rust refuses with %q", test.Error)
				}
				var usage *UsageError
				if !errors.As(err, &usage) {
					t.Fatalf("the refusal is a %T, not a *UsageError", err)
				}
				if err.Error() != test.Error {
					t.Errorf("got %q\nwant %q", err.Error(), test.Error)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			want := numbersAsFloats(mustDecodeValue(t, test.Value))
			if got := parsedValue(t, parsed); !reflect.DeepEqual(got, want) {
				t.Errorf("got %s\nwant %s", encodeValue(got), test.Value)
			}
		})
	}
}

func TestHelpIsRenderedAsTheRustRendersIt(t *testing.T) {
	var trees struct {
		Trees map[string]jsontext.Value `json:"trees"`
	}
	readTable(t, "lines.json", &trees)
	var table []struct {
		Tree string `json:"tree"`
		Help string `json:"help"`
	}
	readTable(t, "help.json", &table)
	for _, test := range table {
		node, err := DecodeNode(trees.Trees[test.Tree])
		if err != nil {
			t.Fatal(err)
		}
		if got := Help(node, test.Tree); got != test.Help {
			t.Errorf("%s\n got:\n%s\nwant:\n%s", test.Tree, got, test.Help)
		}
	}
}

func TestArgumentsAreCheckedAsTheRustChecksThem(t *testing.T) {
	var table []struct {
		Name      string         `json:"name"`
		Leaf      jsontext.Value `json:"leaf"`
		Arguments jsontext.Value `json:"arguments"`
		Want      string         `json:"want"`
	}
	readTable(t, "arguments.json", &table)
	for _, test := range table {
		node, err := DecodeNode(test.Leaf)
		if err != nil {
			t.Fatal(err)
		}
		arguments, ok := mustDecodeValue(t, test.Arguments).(Object)
		if !ok {
			t.Fatalf("%s: arguments are not an object", test.Name)
		}
		got := ""
		if err := node.(Leaf).CheckArguments(arguments); err != nil {
			var usage *UsageError
			if !errors.As(err, &usage) {
				t.Fatalf("%s: the refusal is a %T, not a *UsageError", test.Name, err)
			}
			got = err.Error()
		}
		if got != test.Want {
			t.Errorf("%s\n got: %q\nwant: %q", test.Name, got, test.Want)
		}
	}
}

func TestTheParagraphOfDefaultsIsTheRusts(t *testing.T) {
	want, err := os.ReadFile(filepath.Join("testdata", "rust", "help_defaults.txt"))
	if err != nil {
		t.Fatal(err)
	}
	if HelpDefaults != string(want) {
		t.Errorf("HelpDefaults = %q\nwant %q", HelpDefaults, want)
	}
}
