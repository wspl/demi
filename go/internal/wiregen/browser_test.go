package wiregen

import (
	"encoding/json/v2"
	"strings"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

// The environment contract uses null to remove one variable. Neither emitter
// may turn that into a missing key, nor drop the rules on non-null values.
// Pure generation and schema evaluation; expected cost is under one second.
func TestNullableMapValues(t *testing.T) {
	source := `package p
//demi:enum
type Mode string
const On Mode="on"
const Off Mode="off"
//demi:wire
type Environment struct {
 Values map[string]*string ` + "`json:\"values\" check:\"each(chars=1..4)\"`" + `
 Modes map[string]*Mode ` + "`json:\"modes\"`" + `
}`
	pkg, err := LoadSource(map[string][]byte{"env.go": []byte(source)})
	if err != nil {
		t.Fatal(err)
	}
	document, err := pkg.schemaOf("Environment")
	if err != nil {
		t.Fatal(err)
	}
	raw, err := document.marshal()
	if err != nil {
		t.Fatal(err)
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	compiler := jsonschema.NewCompiler()
	if err := compiler.AddResource("env.json", value); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile("env.json")
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range []struct {
		input    string
		accepted bool
	}{
		{`{"values":{"REMOVE":null,"SET":"a"},"modes":{"REMOVE":null,"SET":"on"}}`, true},
		{`{"values":{},"modes":{}}`, true},
		{`{"values":{"SET":""},"modes":{}}`, false},
		{`{"values":{"SET":"12345"},"modes":{}}`, false},
		{`{"values":{"SET":false},"modes":{}}`, false},
		{`{"values":{},"modes":{"SET":"other"}}`, false},
	} {
		var input any
		if err := json.Unmarshal([]byte(row.input), &input); err != nil {
			t.Fatal(err)
		}
		if err := schema.Validate(input); (err == nil) != row.accepted {
			t.Errorf("%s: %v", row.input, err)
		}
	}
	ts, err := pkg.GenerateTypeScript()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(ts), `z.record(z.string(), z.string().min(1).max(4).nullable())`) || !strings.Contains(string(ts), `z.enum(["on", "off"]).nullable()`) {
		t.Fatalf("TypeScript lost nullable map values:\n%s", ts)
	}
	browser, err := GenerateBrowserTypeScript(map[string]*Package{"p": pkg}, []BrowserRoot{{Type: BrowserType{Package: "p", Name: "Environment"}}}, BrowserOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(browser), `z.record(z.string(), modeSchema.nullable())`) {
		t.Fatalf("browser named map values lost null:\n%s", browser)
	}
}

func TestBrowserScalarFormats(t *testing.T) {
	source := `package p
//demi:opaque
type Scalar struct{}
//demi:wire
type Input struct { Value Scalar ` + "`json:\"value\" check:\"func=Validate\"`" + ` }`
	pkg, err := LoadSource(map[string][]byte{"input.go": []byte(source)})
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range []struct{ format, want string }{
		{"http-url", `z.url({ protocol: z.regexes.httpProtocol })`},
		{"trimmed", `z.string().trim()`},
		{"email", `z.email()`},
		{"date-time", `z.iso.datetime({ precision: 3 })`},
	} {
		t.Run(row.format, func(t *testing.T) {
			scalar := BrowserScalarSchema{Type: "string", Format: row.format, Inline: true}
			schema, err := scalar.JSONSchema()
			if err != nil {
				t.Fatal(err)
			}
			want := `{"type":"string","format":"` + row.format + `"}`
			if string(schema) != want {
				t.Fatalf("JSON Schema: %s, want %s", schema, want)
			}
			options := BrowserOptions{ScalarSchemas: map[BrowserType]BrowserScalarSchema{{Package: "p", Name: "Scalar"}: scalar}}
			ts, err := GenerateBrowserTypeScript(map[string]*Package{"p": pkg}, []BrowserRoot{{Type: BrowserType{Package: "p", Name: "Input"}}}, options)
			if err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(string(ts), "value: "+row.want+",") {
				t.Fatalf("TypeScript lost %s:\n%s", row.format, ts)
			}
		})
	}
	if _, err := (BrowserScalarSchema{Type: "string", Format: "unknown-format"}).JSONSchema(); err == nil {
		t.Fatal("unknown format was silently ignored")
	}
}

// Native G7 metadata must preserve the boundary's three states and unknown
// JSON members, including when an opaque scalar supplies its own string codec.
// In-memory generation and validation; no processes or deadlines.
func TestNativeBrowserMembers(t *testing.T) {
	source := `package p
import "github.com/wspl/demi/go/internal/wire"
//demi:opaque string format=http-url
type Endpoint struct{}
//demi:wire
type Input struct {
 Setting **string ` + "`json:\"setting,omitzero\" check:\"nullable,chars=1..\"`" + `
 Default *string ` + "`json:\"default,omitzero\" check:\"nullabsent\"`" + `
 Endpoint Endpoint ` + "`json:\"endpoint\" check:\"func=Validate\"`" + `
 Extra wire.Members ` + "`json:\",inline\"`" + `
}`
	pkg, err := LoadSource(map[string][]byte{"input.go": []byte(source)})
	if err != nil {
		t.Fatal(err)
	}
	raw, err := pkg.GenerateJSONSchema("Input", nil)
	if err != nil {
		t.Fatal(err)
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	compiler := jsonschema.NewCompiler()
	if err := compiler.AddResource("native.json", value); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile("native.json")
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range []struct {
		input string
		valid bool
	}{
		{`{"endpoint":"https://example.test/","extra":{"future":true}}`, true},
		{`{"endpoint":"https://example.test/","setting":null,"default":null}`, true},
		{`{"endpoint":"https://example.test/","setting":"value"}`, true},
		{`{"endpoint":"https://example.test/","setting":""}`, false},
		{`{"endpoint":"https://example.test/","setting":false}`, false},
		{`{"setting":null}`, false},
	} {
		var input any
		if err := json.Unmarshal([]byte(row.input), &input); err != nil {
			t.Fatal(err)
		}
		if err := schema.Validate(input); (err == nil) != row.valid {
			t.Errorf("%s: %v", row.input, err)
		}
	}
	ordinary, err := pkg.GenerateTypeScript()
	if err != nil {
		t.Fatal(err)
	}
	browser, err := GenerateBrowserTypeScript(map[string]*Package{"p": pkg}, []BrowserRoot{{Type: BrowserType{Package: "p", Name: "Input"}}}, BrowserOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, output := range [][]byte{ordinary, browser} {
		for _, want := range []string{`setting: z.string().min(1).nullable().optional()`, `default: z.string().nullable().optional()`, `endpoint: z.url({ protocol: z.regexes.httpProtocol })`, `.catchall(z.json())`} {
			if !strings.Contains(string(output), want) {
				t.Errorf("missing %s in %s", want, output)
			}
		}
	}
}

// Foreign closed sets and tagged variants remain their owner's contract in
// both browser emitters; no receiving package redeclares the allowed values.
func TestForeignBrowserSchemas(t *testing.T) {
	owner, err := LoadSource(map[string][]byte{"owner.go": []byte(`package owner
//demi:enum
//demi:export
type Mode string
const On Mode="on"
const Off Mode="off"
//demi:union tag=type
//demi:export
type Answer interface{answer()}
//demi:variant ready
type Ready struct{}
func(Ready)answer(){}
`)})
	if err != nil {
		t.Fatal(err)
	}
	consumer, err := loadSource(map[string][]byte{"consumer.go": []byte(`package consumer
import "example.test/owner"
//demi:wire
type Envelope struct {
 Mode owner.Mode ` + "`json:\"mode\" check:\"func=owner.ValidateMode\"`" + `
 Answer owner.Answer ` + "`json:\"answer\" check:\"func=owner.ValidateAnswer\"`" + `
}`)}, func(string) (*Package, error) { return owner, nil })
	if err != nil {
		t.Fatal(err)
	}
	raw, err := consumer.GenerateJSONSchema("Envelope", map[string]*Package{"owner": owner})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), `"enum":["on","off"]`) || !strings.Contains(string(raw), `"const":"ready"`) {
		t.Fatalf("lost owner rules: %s", raw)
	}
	ts, err := GenerateBrowserTypeScript(map[string]*Package{"owner": owner, "consumer": consumer}, []BrowserRoot{{Type: BrowserType{Package: "consumer", Name: "Envelope"}}}, BrowserOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`z.enum(["on", "off"])`, `type: z.literal("ready")`, `mode: modeSchema`, `answer: answerSchema`} {
		if !strings.Contains(string(ts), want) {
			t.Fatalf("lost %s: %s", want, ts)
		}
	}
}
