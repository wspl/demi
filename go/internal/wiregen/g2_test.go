package wiregen

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The declarations of what serde gave the Rust, whose generated code the golden
// files hold: one case for each feature of the contract of a socket.
var serdeGoldenCases = map[string]string{
	"adjacent": `package p

//demi:union tag=op content=params
type Command interface{ command() }

//demi:variant stop open
type Stop struct{}

//demi:variant resize
type Resize struct {
	Bytes uint64 ` + "`json:\"bytes\" check:\"range=1..\"`" + `
	Label *string ` + "`json:\"label,omitzero\"`" + `
}

func (Stop) command()   {}
func (Resize) command() {}

//demi:wire
type Holder struct {
	Command Command ` + "`json:\"command\"`" + `
	Others  []Command ` + "`json:\"others\"`" + `
}
`,
	"inline": `package p

//demi:union tag=op content=params
type Command interface{ command() }

//demi:variant stop
type Stop struct{}

func (Stop) command() {}

// A Request has an id and the members of a command.
//
//demi:wire open
type Request struct {
	ID   string  ` + "`json:\"id\" check:\"chars=1..\"`" + `
	Call Command ` + "`json:\",inline\"`" + `
	Tag  *string ` + "`json:\"tag,omitzero\"`" + `
}
`,
	"foreign": `package p

import (
	"example.com/other"
	alias "example.com/another"
)

//demi:wire
type Fleet struct {
	One  other.Boot            ` + "`json:\"one\" check:\"func=other.Validate\"`" + `
	Many []alias.Boot          ` + "`json:\"many\" check:\"each(func=alias.Validate)\"`" + `
	ByID map[string]other.Boot ` + "`json:\"byId\" check:\"each(func=local)\"`" + `
	Opt  *other.Boot           ` + "`json:\"opt,omitzero\" check:\"func=other.Validate\"`" + `
}

func local(other.Boot) error { return nil }
`,
}

func TestGeneratedGoOfTheSerdeFeaturesIsWhatTheDeclarationsSay(t *testing.T) {
	for name, source := range serdeGoldenCases {
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

// The files the packages of the socket commit are what go generate writes.
func TestCommittedGeneratedFilesOfTheSocketAreCurrent(t *testing.T) {
	for _, dir := range []string{"serdetest", "../../machinesproto", "../../runnerproto"} {
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

func TestADeclarationOfTheSerdeFeaturesThatBreaksTheirRulesIsRefusedNamingIt(t *testing.T) {
	const header = "package p\n\nimport \"example.com/other\"\n\n"
	const union = "//demi:union tag=op content=params\ntype U interface{ u() }\n//demi:variant v\ntype V struct{}\nfunc (V) u() {}\n"
	const tagged = "//demi:union tag=op\ntype U interface{ u() }\n//demi:variant v\ntype V struct{}\nfunc (V) u() {}\n"
	field := func(declaration string) string {
		return header + "//demi:wire\ntype T struct {\n" + declaration + "\n}\n"
	}
	for name, test := range map[string]struct{ source, want string }{
		"a variant that is open in another word":     {header + union + "//demi:variant w closed\ntype W struct{}\nfunc (W) u() {}\n", "takes at most a tag, and `open` or `opaque` after it"},
		"content without a tag":                      {header + "//demi:union content=params\ntype T interface{ t() }\n", "`tag=NAME` may be followed by `content=NAME`"},
		"content after untagged":                     {header + "//demi:union untagged content=params\ntype T interface{ t() }\n", "`tag=NAME` may be followed by `content=NAME`"},
		"a content that is the tag":                  {header + "//demi:union tag=op content=op\ntype T interface{ t() }\n", "names its tag and its content, apart"},
		"an empty content":                           {header + "//demi:union tag=op content=\ntype T interface{ t() }\n", "names its tag and its content, apart"},
		"an inline field that is not a union":        {field("A string `json:\",inline\"`"), "an inline field is an adjacently tagged union"},
		"an inline field of a tagged union":          {header + tagged + "//demi:wire\ntype T struct {\nA U `json:\",inline\"`\n}\n", "an inline field is an adjacently tagged union"},
		"an inline field with a name":                {header + union + "//demi:wire\ntype T struct {\nA U `json:\"a,inline\"`\n}\n", "an inline field has no json name"},
		"an inline field that is a pointer":          {header + union + "//demi:wire\ntype T struct {\nA *U `json:\",inline\"`\n}\n", "an inline field is an adjacently tagged union"},
		"an inline field that is optional":           {header + union + "//demi:wire\ntype T struct {\nA U `json:\",inline,omitzero\"`\n}\n", "an inline field is required"},
		"two inline fields":                          {header + union + "//demi:wire\ntype T struct {\nA U `json:\",inline\"`\nB U `json:\",inline\"`\n}\n", "a struct has one inline field"},
		"a member of the inline tag":                 {header + union + "//demi:wire\ntype T struct {\nA U `json:\",inline\"`\nOp string `json:\"op\"`\n}\n", "the member is one that the inline union U writes"},
		"a member of the inline content":             {header + union + "//demi:wire\ntype T struct {\nParams string `json:\"params\"`\nA U `json:\",inline\"`\n}\n", "the member is one that the inline union U writes"},
		"a wire struct of another package unchecked": {field("A other.Boot `json:\"a\"`"), "the wire struct of another package is checked by its package: name a func= rule"},
		"a foreign struct in a slice unchecked":      {field("A []other.Boot `json:\"a\"`"), "the wire struct of another package is checked by its package: name a func= rule"},
		"a func of a package that is not imported":   {field("A other.Boot `json:\"a\" check:\"func=missing.Validate\"`"), "the file imports no package missing"},
		"a foreign type that is not imported":        {header + "//demi:wire\ntype T struct {\nA missing.Boot `json:\"a\" check:\"func=other.Validate\"`\n}\n", "missing.Boot"},
		"a foreign type as a map key":                {field("A map[other.Key]string `json:\"a\"`"), "the keys are strings"},
	} {
		_, err := LoadSource(map[string][]byte{"p.go": []byte(test.source)})
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Errorf("%s: error = %v, want it to contain %q", name, err, test.want)
		}
	}
}

func TestAnAdjacentUnionOrAnInlineFieldHasNoTypeScriptSchema(t *testing.T) {
	for _, name := range []string{"adjacent", "inline"} {
		pkg, err := LoadSource(map[string][]byte{name + ".go": []byte(serdeGoldenCases[name])})
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if err := pkg.refuseTypeScript(); err == nil {
			t.Errorf("%s: a package with it is written as TypeScript", name)
		}
	}
}

func TestAnAdjacentUnionOrAnInlineFieldHasNoJSONSchema(t *testing.T) {
	for name, want := range map[string]string{
		"adjacent": "an adjacently tagged union has no JSON Schema",
		"inline":   "a struct with an inline union has no JSON Schema",
	} {
		source := strings.Replace(serdeGoldenCases[name], "//demi:wire", "//demi:schema\n//demi:wire", 1)
		if name == "adjacent" {
			source = strings.Replace(source, "//demi:union", "//demi:schema\n//demi:union", 1)
		}
		pkg, err := LoadSource(map[string][]byte{name + ".go": []byte(source)})
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if _, err := pkg.GenerateGo(); err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%s: error = %v, want it to contain %q", name, err, want)
		}
	}
}
