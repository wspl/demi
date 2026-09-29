package commandtree

import (
	"encoding/json/jsontext"
	"errors"
	"fmt"

	"github.com/santhosh-tekuri/jsonschema/v6"

	"github.com/wspl/demi/go/internal/wire"
)

// schemaURL names every schema document while it is compiled, so a relative
// reference resolves against it. It is not a place to fetch anything from:
// [noLoader] refuses every reference to another document.
const schemaURL = "https://demi.invalid/command-schema.json"

// A Schema is a JSON Schema that a declaration carries, compiled once when the
// declaration is read, so every invocation validates against the compiled
// form. The zero Schema is not valid: read one with [NewSchema] or by decoding.
//
// A Schema is immutable and may be shared by every copy of the declaration
// that holds it.
//
//demi:opaque
type Schema struct {
	document Object
	compiled *jsonschema.Schema
}

// NewSchema compiles the JSON Schema document, an object. The schema holds the
// document's top-level members sorted by name, as the Rust holds them in a map,
// so a schema is the same value however it was made and one schema words one
// failure in one order whether it was declared or decoded.
func NewSchema(document Object) (*Schema, error) {
	document = document.sorted()
	compiled, err := compile(document)
	if err != nil {
		return nil, &DeclarationError{Message: err.Error()}
	}
	return &Schema{document: document, compiled: compiled}, nil
}

// compile compiles a schema document. A document is a draft 2020-12 schema
// unless its $schema names another draft.
func compile(document Object) (*jsonschema.Schema, error) {
	compiler := jsonschema.NewCompiler()
	compiler.UseLoader(noLoader{})
	if err := compiler.AddResource(schemaURL, plain(document)); err != nil {
		return nil, err
	}
	return compiler.Compile(schemaURL)
}

// noLoader refuses every schema document that was not added, so a schema never
// reads a file or the network.
type noLoader struct{}

func (noLoader) Load(url string) (any, error) {
	return nil, fmt.Errorf("the schema refers to %s, which a command's schema may not", url)
}

// Document returns the schema: its top-level members sorted by name, and each
// member as the declaration wrote it.
func (s *Schema) Document() Object {
	return s.document
}

// UnmarshalJSONFrom reads a schema: an object that compiles.
func (s *Schema) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	var document Object
	if err := document.UnmarshalJSONFrom(dec); err != nil {
		return err
	}
	document = document.sorted()
	compiled, err := compile(document)
	if err != nil {
		return &wire.InvalidError{Rule: "is not a valid JSON Schema: " + err.Error()}
	}
	*s = Schema{document: document, compiled: compiled}
	return nil
}

// MarshalJSONTo writes the schema: its members sorted by name, and each member
// as it was written.
func (s Schema) MarshalJSONTo(enc *jsontext.Encoder) error {
	return s.document.MarshalJSONTo(enc)
}

// Check reports every way that instance breaks the schema, in one text, or nil
// when it breaks none. The instance is what [DecodeValue] or an [Object]
// holds. Each failure names where it is, such as `"count" is not of type
// "integer"`, rather than repeating the value, which may be a whole stdin body.
// The wording of each failure is the one the model reads, so it is pinned by a
// table of the Rust implementation's messages; the set and the order of the
// failures are the library's, sorted as [Schema.failures] says.
func (s *Schema) Check(instance any) error {
	failures, err := s.failures(instance)
	if err != nil {
		return err
	}
	if len(failures) == 0 {
		return nil
	}
	messages := make([]string, len(failures))
	for i, failure := range failures {
		messages[i] = failure.message(s.document)
	}
	return errors.New(joinFailures(messages))
}
