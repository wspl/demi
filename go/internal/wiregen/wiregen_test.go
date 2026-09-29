package wiregen

import (
	"bytes"
	"encoding/json/v2"
	"flag"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/internal/schematest"
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
	"features": `package p

import "encoding/json/jsontext"

var word = regexp.MustCompile("^[a-z]+$")

type Level string

//demi:enum
//demi:describe How loud.
type Volume string

const (
	VolumeLow  Volume = "low"
	VolumeHigh Volume = "high"
)

//demi:value
//demi:check pattern=word,chars=1..8
//demi:describe A short word.
type Word string

//demi:wire
type Header struct {
	ID string ` + "`json:\"id\"`" + `
}

// A Message has the members of its Header as its own.
//
//demi:wire
//demi:schema
//demi:describe A message.
//demi:describe It has a header.
type Message struct {
	Header
	Body   string  ` + "`json:\"body\"`" + `
	Ratio  float64 ` + "`json:\"ratio\" check:\"range=0.5..4\"`" + `
	Volume Volume  ` + "`json:\"volume\"`" + `
	Words  []Word  ` + "`json:\"words\"`" + `
	Note   *string ` + "`json:\"note\" check:\"nullable\"`" + `
	Extra  jsontext.Value ` + "`json:\"extra\"`" + `
	Loose  Reading ` + "`json:\"loose\"`" + `
	Tree   *Tree   ` + "`json:\"tree,omitzero\"`" + `
}

//demi:wire open
//demi:schema
type Tree struct {
	Children *[]Tree ` + "`json:\"children,omitzero\"`" + `
}

//demi:union untagged
type Reading interface{ reading() }

//demi:variant
//demi:describe A temperature.
type Celsius float64

//demi:variant
type Label string

func (Celsius) reading() {}
func (Label) reading()   {}
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
	for _, dir := range []string{"wiretest", "packtest", "featuretest", "exporttest", "../../runnerproto", "../../commandservice", "../../artifact", "../../claudeproto", "../../commandtree", "../../builtinproto"} {
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
		"a field without a json tag":                 {field("A string"), "T.A: a wire field has a json tag"},
		"a field with an unexported name":            {field("a string `json:\"a\"`"), "T.a: a wire field is exported"},
		"a field named `-`":                          {field("A string `json:\"-\"`"), "T.A: a wire field has a json name"},
		"a json option that is not omitzero":         {field("A string `json:\"a,omitempty\"`"), `the json option "omitempty"`},
		"omitzero on a value":                        {field("A string `json:\"a,omitzero\"`"), "an optional field is a pointer with omitzero"},
		"a pointer that is not optional":             {field("A *string `json:\"a\"`"), "a pointer field is optional and has omitzero"},
		"an embedded type that is not a wire struct": {field("jsontext.Value"), "embeds a wire struct that is not a variant"},
		"a type that is not marked":                  {header + "type U struct{}\n//demi:wire\ntype T struct {\nA U `json:\"a\"`\n}\n", "U is not a type of the wire"},
		"a float32":                                  {field("A float32 `json:\"a\"`"), "float32 is not a type of the wire"},
		"an array":                                   {field("A [2]string `json:\"a\"`"), "use a slice"},
		"a map with keys that are not strings":       {field("A map[int]string `json:\"a\"`"), "the keys are strings"},
		"a slice of pointers":                        {field("A []*string `json:\"a\"`"), "a slice of *string"},
		"a rule that does not apply":                 {field("A bool `json:\"a\" check:\"chars=1..\"`"), "the rule chars does not apply to bool"},
		"an unknown rule":                            {field("A string `json:\"a\" check:\"short\"`"), "unknown rule short"},
		"bounds without a range":                     {field("A string `json:\"a\" check:\"chars=3\"`"), "MIN..MAX"},
		"a bound that is not a number":               {field("A string `json:\"a\" check:\"chars=1..Missing\"`"), "Missing is neither a number nor a constant"},
		"a pattern that the package lacks":           {field("A string `json:\"a\" check:\"pattern=missing\"`"), "the package has no variable missing"},
		"each on a string":                           {field("A string `json:\"a\" check:\"each(nonul)\"`"), "the rule each does not apply to string"},
		"keys on a slice":                            {field("A []string `json:\"a\" check:\"keys(nonul)\"`"), "the rule keys does not apply to []string"},
		"an unclosed parenthesis":                    {field("A []string `json:\"a\" check:\"each(nonul\"`"), "a parenthesis is not closed"},
		"a directive that is unknown":                {header + "//demi:wired\ntype T struct{}\n", "unknown directive"},
		"a wire mark on an interface":                {header + "//demi:wire\ntype T interface{}\n", "marks a struct"},
		"an opaque mark on an interface":             {header + "//demi:opaque\ntype T interface{}\n", "marks a struct"},
		"a pointer to a pointer":                     {field("A **string `json:\"a,omitzero\"`"), "a double pointer requires omitzero and nullable"},
		"a union without a kind":                     {header + "//demi:union\ntype T interface{ t() }\n", "`tag=NAME` or `untagged`"},
		"a union with two methods":                   {header + "//demi:union untagged\ntype T interface{ t(); u() }\n", "one unexported method"},
		"a union without variants":                   {header + "//demi:union untagged\ntype T interface{ t() }\n", "the union has no variants"},
		"a variant of no union":                      {header + "//demi:variant\ntype V struct{}\n", "a variant that implements no union's method"},
		"a variant that is not marked":               {header + "//demi:union untagged\ntype T interface{ t() }\ntype V struct{}\nfunc (V) t() {}\n", "the union has no variants"},
		"a variant of a tagged union without one":    {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant\ntype V struct{}\nfunc (V) t() {}\n", "names its tag"},
		"a variant with a member of its tag":         {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant v\ntype V struct {\nK string `json:\"k\"`\n}\nfunc (V) t() {}\n", "the member is the tag of the union"},
		"two variants with one tag":                  {header + "//demi:union tag=k\ntype T interface{ t() }\n//demi:variant v\ntype V struct{}\n//demi:variant v\ntype W struct{}\nfunc (V) t() {}\nfunc (W) t() {}\n", "have the tag"},
		"an enum without constants":                  {header + "//demi:enum\ntype E string\n", "an enum has constants of its type"},
		"an enum with rules of its own":              {header + "//demi:enum\n//demi:check nonul\ntype E string\nconst A E = \"a\"\n", "an enum has no //demi:check"},
		"an enum with a value twice":                 {header + "//demi:enum\ntype E string\nconst (\nA E = \"a\"\nB E = \"a\"\n)\n", "two constants have the value \"a\""},
		"an enum that is not a string":               {header + "//demi:enum\ntype E int\n", "marks a type named after string"},
		"a value without rules":                      {header + "//demi:value\ntype V string\n", "a value has the rules of its type"},
		"a value with a rule that does not apply":    {header + "//demi:value\n//demi:check range=1..2\ntype V string\n", "the rule range does not apply to V"},
		"rules on a type that is not a value":        {header + "//demi:check nonul\ntype V string\n", "is a wire declaration"},
		"a description on an unmarked type":          {header + "//demi:describe x\ntype V struct{}\n", "is a wire declaration"},
		"a schema on a value":                        {header + "//demi:value\n//demi:check nonul\n//demi:schema\ntype V string\n", "marks a struct or a union"},
		"a description on an opaque type":            {header + "//demi:opaque\n//demi:describe x\ntype V struct{}\n", "an opaque type is described by its package"},
		"a wire mark with an argument":               {header + "//demi:wire closed\ntype V struct{}\n", "takes no argument here"},
		"a nullable field that is not a pointer":     {field("A string `json:\"a\" check:\"nullable\"`"), "a nullable field is a required pointer"},
		"a nullable field that is optional":          {field("A *string `json:\"a,omitzero\" check:\"nullable\"`"), "a nullable field is a required pointer"},
		"a bound with a fraction on an integer":      {field("A int `json:\"a\" check:\"range=0.5..2\"`"), "the bound 0.5 of int has a fraction"},
		"a float bound that is not a number":         {field("A float64 `json:\"a\" check:\"range=..half\"`"), "half is neither a number nor a constant"},
		"an embedded struct with a tag":              {header + "//demi:wire\ntype H struct{}\n//demi:wire\ntype T struct {\nH `json:\"h\"`\n}\n", "an embedded struct has no tag"},
		"an embedded struct that is not marked":      {header + "type H struct{}\n//demi:wire\ntype T struct {\nH\n}\n", "embeds a wire struct that is not a variant"},
		"a struct that embeds itself":                {header + "//demi:wire\ntype T struct {\nT\n}\n", "embeds itself"},
		"two members with one name":                  {header + "//demi:wire\ntype H struct {\nA string `json:\"a\"`\n}\n//demi:wire\ntype T struct {\nH\nB string `json:\"a\"`\n}\n", `two members are named "a"`},
		"a scalar variant of a tagged union":         {header + "//demi:union tag=k\ntype U interface{ u() }\n//demi:variant\ntype V string\nfunc (V) u() {}\n", "a variant that is not an object belongs to an untagged union"},
		"two scalar variants of one kind":            {header + "//demi:union untagged\ntype U interface{ u() }\n//demi:variant\ntype V string\n//demi:variant\ntype W string\nfunc (V) u() {}\nfunc (W) u() {}\n", "both a JSON string"},
		"a variant named after a type of its own":    {header + "//demi:union untagged\ntype U interface{ u() }\n//demi:variant\ntype V []string\nfunc (V) u() {}\n", "marks a struct, or a type named after a basic type"},
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

// ExampleArgs is the declaration of the type that the Rust test
// declared_argument_types_generate_schemas_inside_the_subset derives a schema
// of; testdata/schema/ExampleArgs.rust.json is what the Rust derived.
const exampleArgs = `package p

//demi:enum
type ExampleMode string

const (
	ExampleModeFast ExampleMode = "fast"
	ExampleModeSlow ExampleMode = "slow"
)

//demi:wire
//demi:schema
type ExampleArgs struct {
	// The file to read
	Path string ` + "`json:\"path\"`" + `
	// How many times
	Count   *uint32      ` + "`json:\"count,omitzero\" check:\"range=1..9\"`" + `
	Mode    *ExampleMode ` + "`json:\"mode,omitzero\"`" + `
	Tags    []string     ` + "`json:\"tags\"`" + `
	Labels  *[]string    ` + "`json:\"labels,omitzero\"`" + `
	NoCache *bool        ` + "`json:\"no-cache,omitzero\"`" + `
}
`

func TestDeclaredArgumentTypesGenerateSchemasInsideTheSubset(t *testing.T) {
	pkg, err := LoadSource(map[string][]byte{"example.go": []byte(exampleArgs)})
	if err != nil {
		t.Fatal(err)
	}
	document, err := pkg.schemaOf("ExampleArgs")
	if err != nil {
		t.Fatal(err)
	}
	generated, err := document.marshal()
	if err != nil {
		t.Fatal(err)
	}
	// It is the schema the Rust derives for the same declaration.
	rust, err := os.ReadFile(filepath.Join("testdata", "schema", "ExampleArgs.rust.json"))
	if err != nil {
		t.Fatal(err)
	}
	want, err := schematest.Normalize(rust)
	if err != nil {
		t.Fatal(err)
	}
	got, err := schematest.Normalize(generated)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Errorf("the schema differs from the Rust's\n got: %s\nwant: %s", got, want)
	}
	// And what the Rust test asserts of it.
	var value map[string]any
	if err := json.Unmarshal(generated, &value); err != nil {
		t.Fatal(err)
	}
	if _, has := value["$schema"]; has {
		t.Error("the schema has a $schema member")
	}
	if strings.Contains(string(generated), "null") {
		t.Errorf("the schema allows null: %s", generated)
	}
	if !reflect.DeepEqual(value["required"], []any{"path", "tags"}) {
		t.Errorf("required = %v", value["required"])
	}
	properties := value["properties"].(map[string]any)
	if maximum := properties["count"].(map[string]any)["maximum"]; maximum != float64(9) {
		t.Errorf("the maximum of count = %v", maximum)
	}
	if values := properties["mode"].(map[string]any)["enum"]; !reflect.DeepEqual(values, []any{"fast", "slow"}) {
		t.Errorf("the values of mode = %v", values)
	}
	var schema commandtree.Schema
	if err := json.Unmarshal(generated, &schema); err != nil {
		t.Fatal(err)
	}
	if err := commandtree.CheckInputSubset(&schema); err != nil {
		t.Errorf("the schema is outside the command input subset: %v", err)
	}
}
