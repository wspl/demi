package wiregen

import (
	"bytes"
	"flag"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var update = flag.Bool("update", false, "rewrite the golden files")

// compare checks got against the golden file at path, or rewrites the file.
func compare(t *testing.T, path string, got []byte) {
	t.Helper()
	if *update {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s differs from the generated output (go test -update rewrites it):\n%s", path, got)
	}
}

// The declarations whose generated code the golden files hold, one for each
// thing the generator writes.
var goldenCases = map[string]string{
	"fields": `package p

import "encoding/json/jsontext"

type Mode string

//demi:wire
type Fields struct {
	Text     string            ` + "`json:\"text\"`" + `
	Flag     bool              ` + "`json:\"flag\"`" + `
	Small    uint8             ` + "`json:\"small\"`" + `
	Signed   int32             ` + "`json:\"signed\"`" + `
	Native   int               ` + "`json:\"native\"`" + `
	Mode     Mode              ` + "`json:\"mode\"`" + `
	Names    []string          ` + "`json:\"names\"`" + `
	ByName   map[string]Fields ` + "`json:\"byName\"`" + `
	Raw      jsontext.Value    ` + "`json:\"raw\"`" + `
	Optional *string           ` + "`json:\"optional,omitzero\"`" + `
	Nested   *Fields           ` + "`json:\"nested,omitzero\"`" + `
}
`,
	"rules": `package p

const Limit = 3

var word = regexp.MustCompile("^[a-z]+$")

//demi:wire
type Rules struct {
	Chars   string            ` + "`json:\"chars\" check:\"chars=1..Limit\"`" + `
	Bytes   string            ` + "`json:\"bytes\" check:\"bytes=..8\"`" + `
	Items   []string          ` + "`json:\"items\" check:\"items=1..,unique,each(chars=2..)\"`" + `
	Number  int               ` + "`json:\"number\" check:\"range=-5..Limit\"`" + `
	Version uint64            ` + "`json:\"version\" check:\"eq=1\"`" + `
	Kind    string            ` + "`json:\"kind\" check:\"oneof=a|b,pattern=word,nonul,func=custom\"`" + `
	Table   map[string]string ` + "`json:\"table\" check:\"keys(oneof=x|y),each(nonul)\"`" + `
	Cross   string            ` + "`json:\"cross\"`" + `
}

func (Rules) check() error { return nil }
`,
	"tagged": `package p

//demi:union tag=kind
type Shape interface{ shape() }

//demi:variant circle
type Circle struct {
	Radius uint8 ` + "`json:\"radius\"`" + `
	Label  *string ` + "`json:\"label,omitzero\"`" + `
}

//demi:variant square
type Square struct{}

func (Circle) shape() {}
func (Square) shape() {}

//demi:wire
type Drawing struct {
	Shapes []Shape ` + "`json:\"shapes\"`" + `
}
`,
	"opaque": `package p

// A Schema decodes itself.
//
//demi:opaque
type Schema struct{}

//demi:wire
type Leaf struct {
	Input       *Schema   ` + "`json:\"input,omitzero\"`" + `
	Schemas     []Schema  ` + "`json:\"schemas\"`" + `
	Positionals *[]string ` + "`json:\"positionals,omitzero\" check:\"each(nonul)\"`" + `
	Labels      *map[string]string ` + "`json:\"labels,omitzero\"`" + `
}
`,
	"untagged": `package p

//demi:union untagged
type Loose interface{ loose() }

//demi:variant
type ByNumber struct {
	Number uint8 ` + "`json:\"number\"`" + `
}

//demi:variant
type ByName struct {
	Name string ` + "`json:\"name\"`" + `
}

func (ByNumber) loose() {}
func (ByName) loose()   {}
`,
}

func TestGeneratedGoIsWhatTheDeclarationsSay(t *testing.T) {
	for name, source := range goldenCases {
		pkg, err := LoadSource(map[string][]byte{name + ".go": []byte(source)})
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		files, err := pkg.GenerateGo()
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		for file, data := range files {
			compare(t, filepath.Join("testdata", "go", name, file+".golden"), data)
		}
	}
}

func TestGeneratedTypeScriptIsWhatTheDeclarationsSay(t *testing.T) {
	for name, dir := range map[string]string{"wiretest": "wiretest", "commandservice": "../../commandservice"} {
		pkg, err := Load(dir)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		source, err := pkg.GenerateTypeScript()
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		compare(t, filepath.Join("testdata", "ts", name+".ts"), source)
	}
}

// The files a package commits are what go generate writes: no declaration
// changed without its generated code.
func TestCommittedGeneratedFilesAreCurrent(t *testing.T) {
	for _, dir := range []string{"wiretest", "../../commandservice", "../../artifact", "../../claudeproto", "../../commandtree"} {
		pkg, err := Load(dir)
		if err != nil {
			t.Fatalf("%s: %v", dir, err)
		}
		files, err := pkg.GenerateGo()
		if err != nil {
			t.Fatalf("%s: %v", dir, err)
		}
		for name, want := range files {
			got, err := os.ReadFile(filepath.Join(dir, name))
			if err != nil {
				t.Errorf("%s: %v", dir, err)
				continue
			}
			if !bytes.Equal(got, want) {
				t.Errorf("%s/%s is not what go generate writes: run go generate in %s", dir, name, dir)
			}
		}
	}
}

func TestAnOpaqueTypeHasNoTypeScriptSchema(t *testing.T) {
	pkg, err := LoadSource(map[string][]byte{"opaque.go": []byte(goldenCases["opaque"])})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := pkg.GenerateTypeScript(); err == nil || !strings.Contains(err.Error(), "Schema is decoded by its package and has no TypeScript schema") {
		t.Errorf("error = %v, want the opaque type named", err)
	}
}

func TestADeclarationThatBreaksTheRulesOfTheWireIsRefusedNamingIt(t *testing.T) {
	const header = "package p\n\nimport \"encoding/json/jsontext\"\n\n"
	field := func(declaration string) string {
		return header + "//demi:wire\ntype T struct {\n" + declaration + "\n}\n"
	}
	for name, test := range map[string]struct{ source, want string }{
		"a field without a json tag":              {field("A string"), "T.A: a wire field has a json tag"},
		"a field with an unexported name":         {field("a string `json:\"a\"`"), "T.a: a wire field is exported"},
		"a field named `-`":                       {field("A string `json:\"-\"`"), "T.A: a wire field has a json name"},
		"a json option that is not omitzero":      {field("A string `json:\"a,omitempty\"`"), `the json option "omitempty"`},
		"omitzero on a value":                     {field("A string `json:\"a,omitzero\"`"), "an optional field is a pointer with omitzero"},
		"a pointer that is not optional":          {field("A *string `json:\"a\"`"), "a pointer field is optional and has omitzero"},
		"an embedded field":                       {field("jsontext.Value"), "declares one field per line, and embeds nothing"},
		"a type that is not marked":               {header + "type U struct{}\n//demi:wire\ntype T struct {\nA U `json:\"a\"`\n}\n", "U is not a type of the wire"},
		"a float":                                 {field("A float64 `json:\"a\"`"), "float64 is not a type of the wire"},
		"an array":                                {field("A [2]string `json:\"a\"`"), "use a slice"},
		"a map with keys that are not strings":    {field("A map[int]string `json:\"a\"`"), "the keys are strings"},
		"a slice of pointers":                     {field("A []*string `json:\"a\"`"), "a slice of *string"},
		"a rule that does not apply":              {field("A bool `json:\"a\" check:\"chars=1..\"`"), "the rule chars does not apply to bool"},
		"an unknown rule":                         {field("A string `json:\"a\" check:\"short\"`"), "unknown rule short"},
		"bounds without a range":                  {field("A string `json:\"a\" check:\"chars=3\"`"), "MIN..MAX"},
		"a bound that is not a number":            {field("A string `json:\"a\" check:\"chars=1..Missing\"`"), "Missing is neither a number nor a constant"},
		"a pattern that the package lacks":        {field("A string `json:\"a\" check:\"pattern=missing\"`"), "the package has no variable missing"},
		"each on a string":                        {field("A string `json:\"a\" check:\"each(nonul)\"`"), "the rule each does not apply to string"},
		"keys on a slice":                         {field("A []string `json:\"a\" check:\"keys(nonul)\"`"), "the rule keys does not apply to []string"},
		"an unclosed parenthesis":                 {field("A []string `json:\"a\" check:\"each(nonul\"`"), "a parenthesis is not closed"},
		"a directive that is unknown":             {header + "//demi:wired\ntype T struct{}\n", "unknown directive"},
		"a wire mark on an interface":             {header + "//demi:wire\ntype T interface{}\n", "marks a struct"},
		"an opaque mark on an interface":          {header + "//demi:opaque\ntype T interface{}\n", "marks a struct"},
		"a pointer to a pointer":                  {field("A **string `json:\"a,omitzero\"`"), "a pointer to *string"},
		"a union without a kind":                  {header + "//demi:union\ntype T interface{ t() }\n", "`tag=NAME` or `untagged`"},
		"a union with two methods":                {header + "//demi:union untagged\ntype T interface{ t(); u() }\n", "one unexported method"},
		"a union without variants":                {header + "//demi:union untagged\ntype T interface{ t() }\n", "the union has no variants"},
		"a variant of no union":                   {header + "//demi:variant\ntype V struct{}\n", "a variant that implements no union's method"},
		"a variant that is not marked":            {header + "//demi:union untagged\ntype T interface{ t() }\ntype V struct{}\nfunc (V) t() {}\n", "the union has no variants"},
		"a variant of a tagged union without one": {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant\ntype V struct{}\nfunc (V) t() {}\n", "names its tag"},
		"a variant with a member of its tag":      {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant v\ntype V struct {\nK string `json:\"k\"`\n}\nfunc (V) t() {}\n", "the member is the tag of the union"},
		"two variants with one tag":               {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant v\ntype V struct{}\n//demi:variant v\ntype W struct{}\nfunc (V) t() {}\nfunc (W) t() {}\n", "have the tag"},
	} {
		_, err := LoadSource(map[string][]byte{"p.go": []byte(test.source)})
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Errorf("%s: error = %v, want it to contain %q", name, err, test.want)
		}
	}
}

func TestAFileWithWireDeclarationsAndBuildConstraintsIsRefused(t *testing.T) {
	const declaration = "package p\n\n//demi:wire\ntype T struct{}\n"
	for name, test := range map[string]struct{ file, source string }{
		"a build line":         {"t.go", "//go:build !windows\n\n" + declaration},
		"a platform in a name": {"t_windows.go", declaration},
		"a file never built":   {"t.go", "//go:build ignore\n\n" + declaration},
	} {
		dir := t.TempDir()
		if err := os.WriteFile(filepath.Join(dir, test.file), []byte(test.source), 0o644); err != nil {
			t.Fatal(err)
		}
		if _, err := Load(dir); err == nil || !strings.Contains(err.Error(), "has build constraints") {
			t.Errorf("%s: error = %v, want the build constraints named", name, err)
		}
	}
	// A file without wire declarations may have them.
	dir := t.TempDir()
	for file, source := range map[string]string{"t.go": declaration, "t_windows.go": "package p\n"} {
		if err := os.WriteFile(filepath.Join(dir, file), []byte(source), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := Load(dir); err != nil {
		t.Errorf("a platform file without wire declarations: %v", err)
	}
}
