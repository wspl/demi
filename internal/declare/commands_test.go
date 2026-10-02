package declare_test

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/declare"
)

// All command tests are pure in-process tables; each runs in under one second.
func commandSchema(t *testing.T, document string) *declare.Schema {
	t.Helper()
	schema, err := declare.NewSchema(json.RawMessage(document))
	if err != nil {
		t.Fatal(err)
	}
	return schema
}

// commandLeaf constructs a command for argv scenarios, using actual input schemas.
func commandLeaf(t *testing.T, name, schema string, positionals []string, stdin, rest string) *declare.Leaf[declare.Binding] {
	t.Helper()
	leaf := &declare.Leaf[declare.Binding]{Name: name, Summary: name, Kind: &declare.RPC[declare.Binding]{}}
	if schema != "" {
		leaf.Input = commandSchema(t, schema)
	}
	if positionals != nil {
		leaf.Positionals = &positionals
	}
	if stdin != "" {
		leaf.StdinField = &stdin
	}
	if rest != "" {
		leaf.RestField = &rest
	}
	return leaf
}

// filer declares the same commands as the Rust scenario tree.
func filer(t *testing.T) *declare.Group[declare.Binding] {
	t.Helper()
	create := commandLeaf(t, "create", `{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string","maxLength":8,"description":"File content"}},"required":["path","content"],"additionalProperties":false}`, []string{"path"}, "content", "")
	edit := commandLeaf(t, "edit", `{"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"},"occurrence":{"type":"integer","minimum":1}},"required":["path","old","new"],"additionalProperties":false}`, []string{"path"}, "", "")
	measure := commandLeaf(t, "measure", `{"type":"object","properties":{"v":{"type":"array","items":{"type":"number"}},"label":{"type":"array","items":{"type":"string"}},"quiet":{"type":"boolean"}},"required":["v"],"additionalProperties":false}`, nil, "", "")
	forward := commandLeaf(t, "forward", `{"type":"object","properties":{"args":{"type":"array","items":{"type":"string"}}},"required":["args"],"additionalProperties":false}`, nil, "", "args")
	status := commandLeaf(t, "status", `{"type":"object","properties":{"status":{"type":"string","enum":["pending","in_progress","done"]}},"additionalProperties":false}`, nil, "", "")
	list := commandLeaf(t, "list", "", nil, "", "")
	list.Output = &declare.LeafOutput{JSON: commandSchema(t, `{"type":"object","properties":{"files":{"type":"array","items":{"type":"string"}}},"required":["files"],"additionalProperties":false}`)}
	get := commandLeaf(t, "get", `{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}`, []string{"id"}, "", "")
	tree := &declare.Group[declare.Binding]{Name: "filer", Summary: "Create, edit, and list files.", Subcommands: []declare.Node[declare.Binding]{create, edit, measure, forward, status, list, &declare.Group[declare.Binding]{Name: "watch", Summary: "Background pollers.", Subcommands: []declare.Node[declare.Binding]{get}}}}
	if err := tree.Validate(); err != nil {
		t.Fatal(err)
	}
	return tree
}

// readCommand models the runner's select/parse/stdin/check sequence without IO.
func readCommand(tree declare.Node[declare.Binding], argv []string, stdin *string) (*declare.Parsed, error) {
	selected, err := tree.Select(argv)
	if err != nil {
		return nil, err
	}
	parsed, err := selected.Parse(argv)
	if err != nil {
		return nil, err
	}
	if leaf := declare.AsLeaf(selected.Node); leaf != nil && !parsed.Help {
		return parsed.Validate(leaf, stdin)
	}
	return parsed, nil
}

// assertCommand checks the observable filled input or exact refusal.
func assertCommand(t *testing.T, tree declare.Node[declare.Binding], argv []string, stdin *string, wantValues, wantError string) {
	t.Helper()
	result, err := readCommand(tree, argv, stdin)
	if wantError != "" {
		var usage *declare.UsageError
		if !errors.As(err, &usage) || err.Error() != wantError {
			t.Fatalf("%v: got %v, want %s", argv, err, wantError)
		}
		return
	}
	if err != nil {
		t.Fatalf("%v: %v", argv, err)
	}
	encoded, err := json.Marshal(result.Values)
	if err != nil {
		t.Fatal(err)
	}
	var got, want any
	if err := json.Unmarshal(encoded, &got); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(wantValues), &want); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("%v: got %s, want %s", argv, encoded, wantValues)
	}
}

func TestEachFieldTakesItsValueFromItsOneSource(t *testing.T) {
	tree := filer(t)
	body := "body"
	assertCommand(t, tree, []string{"create", "note.txt"}, &body, `{"path":"note.txt","content":"body"}`, "")
	for _, option := range [][]string{{"--content"}, {"--content", "inline"}, {"--content=inline"}} {
		assertCommand(t, tree, append([]string{"create", "note.txt"}, option...), &body, "", `"filer create" reads content only from stdin. Remove --content and use a quoted heredoc, pipe, or input redirection.`)
	}
	assertCommand(t, tree, []string{"create", "note.txt", "inline"}, &body, "", `Unexpected positional argument "inline"`)
	assertCommand(t, tree, []string{"create", "--path", "note.txt"}, &body, "", `"path" is a positional argument for "filer create"; --path is not an option`)
	assertCommand(t, tree, []string{"create", "--", "--help"}, &body, `{"path":"--help","content":"body"}`, "")
	assertCommand(t, tree, []string{"forward", "--", "--help", "--json"}, nil, `{"args":["--help","--json"]}`, "")
	assertCommand(t, tree, []string{"forward", "--args", "value"}, nil, "", `"args" is passed after -- for "filer forward"; --args is not an option`)
	help := tree.Help("filer")
	for _, want := range []string{"  filer create <path> <<'EOF'\n  <content>\n  EOF\n", "    Stdin body: content - File content", "  filer forward -- <args>...\n"} {
		if !strings.Contains(help, want) {
			t.Errorf("help lacks %q", want)
		}
	}
	for _, option := range []string{"--path", "--content", "--args"} {
		if strings.Contains(help, option) {
			t.Errorf("help contains %s", option)
		}
	}
}

func TestAnOptionValueNeverSwallowsTheNextOption(t *testing.T) {
	tree := filer(t)
	assertCommand(t, tree, []string{"edit", "note.txt", "--old", "--new", "replacement"}, nil, "", `Missing value for "--old"`)
	assertCommand(t, tree, []string{"edit", "note.txt", "--old=--help", "--new="}, nil, `{"path":"note.txt","old":"--help","new":""}`, "")
	assertCommand(t, tree, []string{"edit", "note.txt", "--old", "a", "--old", "b", "--new", "c"}, nil, "", `Duplicate value for "old"`)
}

func TestArgvTextBecomesDeclaredValuesAndOneRefusalNamesEveryFailure(t *testing.T) {
	tree := filer(t)
	for _, test := range []struct {
		argv []string
		want string
	}{
		{[]string{"measure", "--v", "12", "--v", "13"}, `{"v":[12,13]}`},
		{[]string{"measure", "--v", "1.5", "--label", "a", "--quiet"}, `{"v":[1.5],"label":["a"],"quiet":true}`},
		{[]string{"measure", "--v", "1", "--quiet=false"}, `{"v":[1],"quiet":false}`},
		{[]string{"edit", "f", "--old", "a", "--new", "b", "--occurrence", "2"}, `{"path":"f","old":"a","new":"b","occurrence":2}`},
	} {
		assertCommand(t, tree, test.argv, nil, test.want, "")
	}
	for _, test := range []struct {
		argv     []string
		failures []string
	}{
		{[]string{"measure", "--v", "twelve", "--quiet=maybe"}, []string{`"v.0" is not of type "number"`, `"quiet" is not of type "boolean"`}},
		{[]string{"edit", "f", "--occurrence", "NaN"}, []string{`"occurrence" is not of type "integer"`, `"old" is a required property`, `"new" is a required property`}},
	} {
		_, err := readCommand(tree, test.argv, nil)
		if err == nil || !strings.HasPrefix(err.Error(), "Invalid command arguments: ") {
			t.Fatalf("got %v", err)
		}
		for _, want := range test.failures {
			if !strings.Contains(err.Error(), want) {
				t.Errorf("%s lacks %s", err, want)
			}
		}
		if strings.Contains(err.Error(), "twelve") || strings.Contains(err.Error(), "maybe") {
			t.Errorf("disclosed input: %s", err)
		}
	}
	body := "a long body"
	assertCommand(t, tree, []string{"create", "note.txt"}, &body, "", `Invalid command arguments: "content" is longer than 8 characters`)
}

func TestACommandIsFoundAndNamedByItsFullPath(t *testing.T) {
	root := commandLeaf(t, "kcenv", `{"type":"object","properties":{"key":{"type":"string"}},"required":["key"],"additionalProperties":false}`, []string{"key"}, "", "")
	result, err := readCommand(root, []string{"HOME"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(result.Path, []string{"kcenv"}) || result.Help || result.JSON {
		t.Fatalf("got %+v", result)
	}
	assertCommand(t, root, []string{"HOME"}, nil, `{"key":"HOME"}`, "")
	tree := filer(t)
	result, err = readCommand(tree, []string{"watch"}, nil)
	if err != nil || !result.Help || !reflect.DeepEqual(result.Path, []string{"filer", "watch"}) {
		t.Fatalf("got %+v, %v", result, err)
	}
	assertCommand(t, tree, []string{"watch", "get", "my-id"}, nil, `{"id":"my-id"}`, "")
	assertCommand(t, tree, []string{"watch", "missing"}, nil, "", `Unknown subcommand "filer watch missing"`)
	assertCommand(t, tree, []string{"watch", "get", "my-id", "--missing", "x"}, nil, "", `Unknown option "--missing" for "filer watch get"`)
}

func TestJSONOutputIsOfferedAndAcceptedOnlyWhereDeclared(t *testing.T) {
	tree := filer(t)
	help := tree.Help("filer")
	for _, want := range []string{"  filer status [--status <pending|in_progress|done>]\n", "  filer list [--json]\n"} {
		if !strings.Contains(help, want) {
			t.Errorf("help lacks %q", want)
		}
	}
	parsed, err := readCommand(tree, []string{"list", "--json"}, nil)
	if err != nil || !parsed.JSON {
		t.Fatalf("got %+v, %v", parsed, err)
	}
	assertCommand(t, tree, []string{"status", "--json"}, nil, "", `Command "filer status" does not define JSON output`)
}

func TestALeafNamesValidInputs(t *testing.T) {
	for _, test := range []struct {
		schema string
		valid  bool
	}{
		{`{"type":"object","properties":{"text":{"type":"string"}}}`, true},
		{`{"type":"array"}`, false},
		{`{"type":"object","properties":{"json":{"type":"boolean"}}}`, false},
		{`{"type":"object","properties":{"bad name":{"type":"string"}}}`, false},
	} {
		leaf := commandLeaf(t, "add", test.schema, nil, "", "")
		if err := leaf.Validate(); (err == nil) != test.valid {
			t.Errorf("%s: %v", test.schema, err)
		}
	}
}

func TestALeafRunsOneWay(t *testing.T) {
	t.Skip("needs f-contractgen3: untagged unions")
	// Once generated codecs exist, this table runs through their NodeJSON holder.
	for _, test := range []struct {
		document string
		valid    bool
	}{
		{`{"name":"add","summary":"Add","kind":"rpc"}`, true},
		{`{"name":"add","summary":"Add","kind":"rpc","binding":{"package":"demi.file","operation":"file.read","descriptorHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}`, false},
		{`{"name":"read","summary":"Read","kind":"native"}`, false},
	} {
		var node declare.Node[declare.Binding]
		err := json.Unmarshal([]byte(test.document), &node)
		if (err == nil) != test.valid {
			t.Errorf("%s: %v", test.document, err)
		}
	}
}

func TestGroupsNameDistinctSubcommands(t *testing.T) {
	leaf := commandLeaf(t, "add", "", nil, "", "")
	for _, test := range []struct {
		children []declare.Node[declare.Binding]
		valid    bool
	}{
		{[]declare.Node[declare.Binding]{leaf}, true},
		{[]declare.Node[declare.Binding]{leaf, leaf}, false},
		{[]declare.Node[declare.Binding]{}, false},
	} {
		group := &declare.Group[declare.Binding]{Name: "todo", Summary: "Todos", Subcommands: test.children}
		if err := group.Validate(); (err == nil) != test.valid {
			t.Errorf("got %v", err)
		}
	}
}

// TestHelpAndCommandLinesMatchRecordedCases executes the Rust fixture with a
// plain Go tree; only decoding the manifest itself awaits generated codecs.
func TestHelpAndCommandLinesMatchRecordedCases(t *testing.T) {
	data, err := os.ReadFile("testdata/cli.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Help  string
		Cases []struct {
			Argv    []string
			Stdin   *string
			Invalid bool
			Parsed  json.RawMessage
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	leaf := commandLeaf(t, "read", `{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"path":{"type":"string","description":"File path"},"count":{"default":2,"description":"Count","type":"integer","exclusiveMinimum":0,"maximum":9007199254740991},"upper":{"description":"Uppercase","type":"boolean"},"tag":{"description":"Tags","type":"array","items":{"type":"string"}},"body":{"type":"string","description":"Text body"},"args":{"description":"Forwarded arguments","type":"array","items":{"type":"string"}}},"required":["path","body"]}`, []string{"path"}, "body", "args")
	leaf.Summary = "Read a native file."
	leaf.Kind = &declare.Native[declare.Binding]{Binding: declare.Binding{Package: "demicodes.fixture", Operation: "file.read", DescriptorHash: "7d320ed041df0a3277ce2946c2893522cac021e30c3876dc2cb58ac29bfbe636"}}
	leaf.Output = &declare.LeafOutput{JSON: commandSchema(t, `{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}`)}
	tree := &declare.Group[declare.Binding]{Name: "fixture", Summary: "CLI fixture.", Subcommands: []declare.Node[declare.Binding]{leaf}}
	if err := tree.Validate(); err != nil {
		t.Fatal(err)
	}
	if got := tree.Help("fixture"); got != fixture.Help {
		t.Fatalf("help differs\ngot:\n%s\nwant:\n%s", got, fixture.Help)
	}
	for i, test := range fixture.Cases {
		t.Run(fmt.Sprint(i), func(t *testing.T) {
			result, err := readCommand(tree, test.Argv, test.Stdin)
			if test.Invalid {
				if err == nil {
					t.Fatal("invalid argv accepted")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := json.Marshal(result)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			if err := json.Unmarshal(encoded, &got); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(test.Parsed, &want); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("got %s, want %s", encoded, test.Parsed)
			}
		})
	}
}

func TestManifestFixtureDecodes(t *testing.T) {
	t.Skip("needs f-contractgen3: untagged unions")
	data, err := os.ReadFile("testdata/cli.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture map[string]json.RawMessage
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	var manifest map[string]json.RawMessage
	if err := json.Unmarshal(fixture["manifest"], &manifest); err != nil {
		t.Fatal(err)
	}
	var roots map[string]map[string]json.RawMessage
	if err := json.Unmarshal(manifest["roots"], &roots); err != nil {
		t.Fatal(err)
	}
	var node declare.Node[declare.Binding]
	if err := json.Unmarshal(roots["fixture"]["tree"], &node); err != nil {
		t.Fatal(err)
	}
	if err := node.Validate(); err != nil {
		t.Fatal(err)
	}
	var help string
	if err := json.Unmarshal(fixture["help"], &help); err != nil {
		t.Fatal(err)
	}
	if node.Help("fixture") != help {
		t.Fatal("decoded manifest help differs")
	}
}

func TestPinningPreservesDeclarationsAndPropagatesResolutionFailures(t *testing.T) {
	hint := "Running"
	positionals := []string{"key"}
	original := &declare.Leaf[declare.NativeOperation]{Name: "read", Summary: "Read", Input: commandSchema(t, `{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}`), Positionals: &positionals, RunningHint: &hint, Kind: &declare.Native[declare.NativeOperation]{Binding: declare.NativeOperation{Package: "demi.file", Operation: "file.read"}}}
	rpc := &declare.Leaf[declare.NativeOperation]{Name: "rpc", Summary: "RPC", Kind: &declare.RPC[declare.NativeOperation]{}}
	root := &declare.Group[declare.NativeOperation]{Name: "files", Summary: "Files", Subcommands: []declare.Node[declare.NativeOperation]{original, rpc}}
	var calls []declare.NativeOperation
	pinned, err := declare.Pin(root, func(operation declare.NativeOperation) (string, error) {
		calls = append(calls, operation)
		return "digest", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	leaves := pinned.Leaves()
	if len(calls) != 1 || calls[0].Operation != "file.read" || len(leaves) != 2 || leaves[0].Binding().DescriptorHash != "digest" || leaves[1].Binding() != nil {
		t.Fatalf("calls=%v, leaves=%v", calls, leaves)
	}
	if pinned.Help("files") != root.Help("files") || declare.Name(pinned) != "files" || declare.Summary(pinned) != "Files" || declare.AsLeaf(pinned) != nil {
		t.Fatal("pin changed declaration")
	}
	*leaves[0].RunningHint = "changed"
	(*leaves[0].Positionals)[0] = "changed"
	if hint != "Running" || positionals[0] != "key" {
		t.Fatal("pin shares mutable declaration metadata")
	}
	failure := errors.New("resolver failed")
	if _, err := declare.Pin(root, func(declare.NativeOperation) (string, error) { return "", failure }); !errors.Is(err, failure) {
		t.Fatalf("lost resolver error: %v", err)
	}
}

func TestDeclarationSourceRulesAndDepth(t *testing.T) {
	for _, test := range []struct {
		name, schema      string
		positionals       []string
		stdin, rest, want string
	}{
		{"bad name", `{}`, nil, "", "", "invalid command name: bad name"},
		{"x", `{"type":"object","properties":{"x":{"type":"string"}}}`, []string{"x"}, "x", "", "multiple input sources for x"},
		{"x", `{"type":"object"}`, []string{"missing"}, "", "", "input source has no schema: missing"},
		{"x", `{"type":"object","properties":{"body":{"type":"integer"}}}`, nil, "body", "", "stdin input must be a string"},
		{"x", `{"type":"object","properties":{"rest":{"type":"array","items":{"type":"integer"}}}}`, nil, "", "rest", "rest input must be a string array"},
		{"x", `{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["b"]}`, []string{"a", "b"}, "", "", "required positional follows optional positional"},
	} {
		leaf := commandLeaf(t, test.name, test.schema, test.positionals, test.stdin, test.rest)
		var declaration *declare.DeclarationError
		if err := leaf.Validate(); !errors.As(err, &declaration) || err.Error() != test.want {
			t.Errorf("got %v, want %s", err, test.want)
		}
	}
	var root declare.Node[declare.Binding] = commandLeaf(t, "leaf", "", nil, "", "")
	for range declare.MaxDepth {
		root = &declare.Group[declare.Binding]{Name: "g", Subcommands: []declare.Node[declare.Binding]{root}}
	}
	if err := root.Validate(); err != nil {
		t.Fatal(err)
	}
	root = &declare.Group[declare.Binding]{Name: "g", Subcommands: []declare.Node[declare.Binding]{root}}
	if err := root.Validate(); err == nil || err.Error() != "command tree exceeds 32 levels" {
		t.Fatalf("got %v", err)
	}
}

func TestHelpSkipsStdinAndArgumentsAreCheckedWithoutConversion(t *testing.T) {
	tree := filer(t)
	parsed, err := readCommand(tree, []string{"create", "--help"}, nil)
	if err != nil || !parsed.Help {
		t.Fatalf("help tried to consume stdin: %v", err)
	}
	assertCommand(t, tree, []string{"create", "f"}, nil, "", "stdin field was not supplied by dispatcher")
	body := ""
	assertCommand(t, tree, []string{"list"}, &body, "", "stdin body supplied to a leaf without stdinField")
	leaf := commandLeaf(t, "count", `{"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]}`, nil, "", "")
	if err := leaf.CheckArguments(json.RawMessage(`{"count":"7"}`)); err == nil || err.Error() != `Invalid command arguments: "count" is not of type "integer"` {
		t.Fatalf("got %v", err)
	}
	if err := leaf.CheckArguments(json.RawMessage(`{"count":7}`)); err != nil {
		t.Fatal(err)
	}
	empty := commandLeaf(t, "empty", "", nil, "", "")
	if err := empty.CheckArguments(json.RawMessage(`{}`)); err != nil {
		t.Fatal(err)
	}
	if err := empty.CheckArguments(json.RawMessage(`{"x":1}`)); err == nil || err.Error() != "Invalid command arguments: the command takes no arguments" {
		t.Fatalf("got %v", err)
	}
}

func TestArgvDiagnosticOrderAndNumericGrammar(t *testing.T) {
	leaf := commandLeaf(t, "number", `{"type":"object","properties":{"a":{"type":"number"},"b":{"type":"boolean"}}}`, nil, "", "")
	assertCommand(t, leaf, []string{"--b=bad", "--a=bad"}, nil, "", `Invalid command arguments: "b" is not of type "boolean"; "a" is not of type "number"`)
	for _, value := range []string{"1_000", "0x1p2", "NaN", "inf", "Infinity", "1e9999"} {
		assertCommand(t, leaf, []string{"--a=" + value}, nil, "", `Invalid command arguments: "a" is not of type "number"`)
	}
	for _, test := range []struct{ value, want string }{{"  +2.0 ", `{"a":2}`}, {"1e2", `{"a":100}`}, {".5", `{"a":0.5}`}, {"-0", `{"a":0}`}} {
		assertCommand(t, leaf, []string{"--a=" + test.value}, nil, test.want, "")
	}
}
