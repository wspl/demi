package commandservice

import (
	"encoding/json/jsontext"
	"reflect"
	"sync"

	"github.com/google/jsonschema-go/jsonschema"
)

// WireValue is the closed set of SDK values accepted by Decode and Encode.
// Its private validation method and exact type set exclude caller-defined types,
// including structs that embed an SDK value.
type WireValue interface {
	validate() error
	Invocation | *Invocation |
		Completion | *Completion |
		ConversationRequest | *ConversationRequest |
		ConversationStatus | *ConversationStatus |
		NumbersOpen | *NumbersOpen |
		NumbersRequest | *NumbersRequest |
		NumbersAnswer | *NumbersAnswer |
		PackageDescriptor | *PackageDescriptor |
		ServiceInfo | *ServiceInfo |
		ArtifactURL | *ArtifactURL |
		ArtifactPath | *ArtifactPath |
		ArtifactLocation | *ArtifactLocation |
		EditContext | *EditContext |
		EditJournal | *EditJournal
}

const conversationPattern = `^[A-Za-z0-9_-]{1,64}$`
const noNULPattern = `^[^\x00]*$`

// wireVariant narrows a derived command tagged-union schema to one alternative.
func wireVariant(s *jsonschema.Schema, tag, value string, required []string, forbidden ...string) *jsonschema.Schema {
	variant := s.CloneSchemas()
	variant.OneOf = nil
	variant.Properties[tag].Enum = []any{value}
	variant.Required = required
	for _, name := range forbidden {
		delete(variant.Properties, name)
	}
	return variant
}

// serviceCatalogSchema applies the shared command catalog contract.
func serviceCatalogSchema(s *jsonschema.Schema) {
	s.Properties["protocolVersion"].Enum = []any{Version}
	operations := s.Properties["operations"]
	operations.MinItems = new(1)
	operations.UniqueItems = true
	operations.Items.MinLength = new(1)
}

var wireSchemas = sync.OnceValues(func() (map[reflect.Type]*jsonschema.Resolved, error) {
	opts := &jsonschema.ForOptions{TypeSchemas: map[reflect.Type]*jsonschema.Schema{
		reflect.TypeFor[jsontext.Value](): {Type: "object"},
		reflect.TypeFor[string]():         {Type: "string", Not: &jsonschema.Schema{Type: "null"}},
		reflect.TypeFor[bool]():           {Type: "boolean", Not: &jsonschema.Schema{Type: "null"}},
		reflect.TypeFor[int64]():          {Type: "integer", Not: &jsonschema.Schema{Type: "null"}},
		reflect.TypeFor[uint8](): {
			Type:    "integer",
			Not:     &jsonschema.Schema{Type: "null"},
			Minimum: new(float64(0)),
			Maximum: new(float64(255)),
		},
		reflect.TypeFor[uint64](): {Type: "integer", Not: &jsonschema.Schema{Type: "null"}, Minimum: new(float64(0))},
	}}
	// Children precede parents so inference applies each nested constraint.
	registrations := []struct {
		typ       reflect.Type
		constrain func(*jsonschema.Schema)
	}{
		{reflect.TypeFor[CommandCaller](), commandCallerSchema},
		{reflect.TypeFor[CommandLocale](), commandLocaleSchema},
		{reflect.TypeFor[CommandContext](), commandContextSchema},
		{reflect.TypeFor[EditContext](), nil},
		{reflect.TypeFor[Invocation](), invocationSchema},
		{reflect.TypeFor[CommandError](), nil},
		{reflect.TypeFor[Completion](), nil},
		{reflect.TypeFor[ConversationRequest](), conversationRequestSchema},
		{reflect.TypeFor[ConversationStatus](), conversationStatusSchema},
		{reflect.TypeFor[Sequence](), sequenceSchema},
		{reflect.TypeFor[NumbersOpen](), nil},
		{reflect.TypeFor[NumbersRequest](), numbersRequestSchema},
		{reflect.TypeFor[NumbersAnswer](), numbersAnswerSchema},
		{reflect.TypeFor[PackageArtifact](), packageArtifactSchema},
		{reflect.TypeFor[PackageDescriptor](), packageDescriptorSchema},
		{reflect.TypeFor[ServiceInfo](), serviceInfoSchema},
		{reflect.TypeFor[ArtifactURL](), nil},
		{reflect.TypeFor[ArtifactPath](), artifactPathSchema},
		{reflect.TypeFor[ArtifactLocation](), artifactLocationSchema},
		{reflect.TypeFor[EditCopies](), editCopiesSchema},
		{reflect.TypeFor[EditKind](), editKindSchema},
		{reflect.TypeFor[EditFile](), editFileSchema},
		{reflect.TypeFor[EditJournal](), editJournalSchema},
	}
	resolved := make(map[reflect.Type]*jsonschema.Resolved, len(registrations))
	for _, registration := range registrations {
		schema, err := jsonschema.ForType(registration.typ, opts)
		if err != nil {
			return nil, err
		}
		// Inference widens pointer schemas to include null. Not survives that widening:
		// optional wire fields permit absence, never an explicit null.
		schema.Not = &jsonschema.Schema{Type: "null"}
		if registration.constrain != nil {
			registration.constrain(schema)
		}
		opts.TypeSchemas[registration.typ] = schema
		checked, err := schema.Resolve(nil)
		if err != nil {
			return nil, err
		}
		resolved[registration.typ] = checked
	}
	return resolved, nil
})
