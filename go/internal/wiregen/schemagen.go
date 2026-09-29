package wiregen

import (
	"bytes"
	"encoding/json/v2"
	"fmt"
	"slices"
	"strconv"
	"strings"
)

// schemasFile is the file that holds the JSON Schemas of the package's types.
const schemasFile = "schemas" + generatedSuffix

// schemas writes the JSON Schema of each struct and union marked
// //demi:schema, as a function that returns it, or nothing when the package
// marks none. The settings are those of the Rust command tree
// (crates/command-tree/src/input.rs, command_schema_settings): draft 2020-12
// without a $schema member, every subschema inline, and no optional member that
// allows null. A type that refers to itself is the exception to inline: its
// schema refers to a definition of it under $defs.
func (p *Package) schemas() ([]byte, error) {
	var roots []string
	for _, file := range p.Files {
		for _, name := range file.Types {
			if s, ok := p.Structs[name]; ok && s.Schema {
				roots = append(roots, name)
			}
			if u, ok := p.Unions[name]; ok && u.Schema {
				roots = append(roots, name)
			}
		}
	}
	if len(roots) == 0 {
		return nil, nil
	}
	g := &goGen{imports: map[string]bool{"encoding/json/jsontext": true}}
	var constants goGen
	for _, name := range roots {
		document, err := p.schemaOf(name)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", name, err)
		}
		text, err := document.marshal()
		if err != nil {
			return nil, fmt.Errorf("%s: %w", name, err)
		}
		variable := strings.ToLower(name[:1]) + name[1:] + "Schema"
		g.p("// %sSchema returns the JSON Schema of %s: draft 2020-12 without $schema,", name, name)
		g.p("// with its subschemas inline, and with no optional member that allows null.")
		g.p("func %sSchema() jsontext.Value {", name)
		g.p("return jsontext.Value(%s)", variable)
		g.p("}")
		g.p("")
		constants.p("const %s = %s", variable, strconv.Quote(string(text)))
	}
	g.buf.Write(constants.buf.Bytes())
	return g.finish(p.Name)
}

// A member is one member of a JSON object under construction.
type schemaMember struct {
	key   string
	value any
}

// A schemaObject is a JSON object that keeps the order of its members. A
// schema is built of objects, arrays ([]any), strings, booleans and numbers.
type schemaObject []schemaMember

func (o *schemaObject) add(key string, value any) {
	*o = append(*o, schemaMember{key, value})
}

// marshal writes the object as compact JSON.
func (o schemaObject) marshal() ([]byte, error) {
	var out bytes.Buffer
	if err := writeSchema(&out, o); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

func writeSchema(out *bytes.Buffer, value any) error {
	switch value := value.(type) {
	case schemaObject:
		out.WriteByte('{')
		for i, member := range value {
			if i > 0 {
				out.WriteByte(',')
			}
			key, err := json.Marshal(member.key)
			if err != nil {
				return err
			}
			out.Write(key)
			out.WriteByte(':')
			if err := writeSchema(out, member.value); err != nil {
				return err
			}
		}
		out.WriteByte('}')
	case []any:
		out.WriteByte('[')
		for i, element := range value {
			if i > 0 {
				out.WriteByte(',')
			}
			if err := writeSchema(out, element); err != nil {
				return err
			}
		}
		out.WriteByte(']')
	default:
		encoded, err := json.Marshal(value)
		if err != nil {
			return err
		}
		out.Write(encoded)
	}
	return nil
}

// A schemer builds the JSON Schema of one root.
type schemer struct {
	p              *Package
	owners         map[string]*Package
	foreignPending map[string]bool
	// pending are the structs being expanded, which a member that refers to one
	// of them refers to by $ref.
	pending map[string]bool
	// refs are the structs that were referred to, in the order they were.
	refs []string
}

// schemaOf returns the JSON Schema of the struct or union name.
func (p *Package) schemaOf(name string) (schemaObject, error) {
	return p.schemaWithOwners(name, nil, map[string]bool{})
}

// GenerateJSONSchema emits a root with the declarations of its foreign owners.
func (p *Package) GenerateJSONSchema(name string, owners map[string]*Package) ([]byte, error) {
	document, err := p.schemaWithOwners(name, owners, map[string]bool{})
	if err != nil {
		return nil, err
	}
	return document.marshal()
}

func (p *Package) schemaWithOwners(name string, owners map[string]*Package, active map[string]bool) (schemaObject, error) {
	key := p.Name + "." + name
	if active[key] {
		return nil, fmt.Errorf("%s: recursive foreign schema is unsupported", key)
	}
	active[key] = true
	defer delete(active, key)
	b := &schemer{p: p, pending: map[string]bool{}, owners: owners, foreignPending: active}
	var document schemaObject
	var err error
	if s, ok := p.Structs[name]; ok {
		document, err = b.structure(s)
		if err == nil && s.Description != "" {
			document.add("description", s.Description)
		}
	} else if t := p.named[name]; t != nil {
		var value any
		value, err = b.value(t, t.Rules, t.Description)
		if err == nil {
			document = value.(schemaObject)
		}
	} else if t := p.OpaqueScalars[name]; t != nil {
		var value any
		value, err = b.value(t, nil, "")
		if err == nil {
			document = value.(schemaObject)
		}
	} else {
		u := p.Unions[name]
		if u == nil {
			return nil, fmt.Errorf("%s: missing schema declaration", name)
		}
		document, err = b.union(u)
		if err == nil && u.Description != "" {
			document.add("description", u.Description)
		}
	}
	if err != nil {
		return nil, err
	}
	document.add("title", name)
	if len(b.refs) == 0 {
		return document, nil
	}
	var definitions schemaObject
	// A definition may refer to a struct that has none yet.
	for i := 0; i < len(b.refs); i++ {
		ref := b.refs[i]
		definition, err := b.definition(ref)
		if err != nil {
			return nil, err
		}
		definitions.add(ref, definition)
	}
	document.add("$defs", definitions)
	return document, nil
}

// definition returns the schema of a struct that refers to itself, which a
// $ref leads to.
func (b *schemer) definition(name string) (schemaObject, error) {
	s := b.p.Structs[name]
	b.pending[name] = true
	defer delete(b.pending, name)
	definition, err := b.structure(s)
	if err != nil {
		return nil, err
	}
	if s.Description != "" {
		definition.add("description", s.Description)
	}
	return definition, nil
}

// structure returns the object schema of a struct, without its description.
func (b *schemer) structure(s *Struct) (schemaObject, error) {
	if hasInline(s) {
		return nil, fmt.Errorf("%s: a struct with an inline union has no JSON Schema", s.Name)
	}
	b.pending[s.Name] = true
	defer delete(b.pending, s.Name)
	var schema schemaObject
	schema.add("type", "object")
	if !s.Open && s.Unknown == nil {
		schema.add("additionalProperties", false)
	}
	var properties schemaObject
	var required []any
	for _, f := range s.Fields {
		property, err := b.field(f)
		if err != nil {
			return nil, fmt.Errorf("%s.%s: %w", s.Name, f.Name, err)
		}
		properties.add(f.JSON, property)
		if f.Required {
			required = append(required, f.JSON)
		}
	}
	if s.Union != nil && s.Union.TagName != "" {
		tag := schemaObject{{"type", "string"}, {"const", s.Tag}}
		properties.add(s.Union.TagName, tag)
		required = slices.Insert(required, 0, any(s.Union.TagName))
	}
	schema.add("properties", properties)
	if len(required) > 0 {
		schema.add("required", required)
	}
	return schema, nil
}

// field returns the schema of a member: what its type is, under its rules, and
// the description of the field, or else of its type.
func (b *schemer) field(f *Field) (any, error) {
	schema, err := b.value(f.Type, f.Rules, f.Doc)
	if err != nil {
		return nil, err
	}
	if f.Nullable || f.NullAsAbsent {
		object, ok := schema.(schemaObject)
		if !ok {
			return nil, fmt.Errorf("a nullable member has an object schema")
		}
		return nullable(object), nil
	}
	return schema, nil
}

// nullable makes a schema with a type allow null.
func nullable(schema schemaObject) schemaObject {
	for _, member := range schema {
		if member.key == "enum" || member.key == "const" {
			return schemaObject{{"anyOf", []any{schema, schemaObject{{"type", "null"}}}}}
		}
	}
	for i, member := range schema {
		if member.key == "type" {
			schema[i].value = []any{member.value, "null"}
			return schema
		}
	}
	return schemaObject{{"anyOf", []any{schema, schemaObject{{"type", "null"}}}}}
}

// value returns the schema of a value of type t under rules. The description
// is the given one, or else the type's own.
func (b *schemer) value(t *Type, rules []Rule, description string) (any, error) {
	if t.Kind == KindPointer {
		return b.value(t.Elem, rules, description)
	}
	if t.WireKind == KindString {
		scalar := BrowserScalarSchema{Type: "string", Format: t.Format}
		if err := scalar.check(); err != nil {
			return nil, err
		}
		schema := stringSchema(b.p, rules)
		if t.Format != "" {
			schema.add("format", t.Format)
		}
		if description != "" {
			schema.add("description", description)
		}
		return schema, nil
	}
	if t.Qualifier != "" {
		owner := b.owners[t.Qualifier]
		if owner == nil {
			for _, file := range b.p.Files {
				if path := file.Imports[t.Qualifier]; path != "" {
					var err error
					owner, err = foreignLoader(".")(path)
					if err != nil {
						return nil, fmt.Errorf("%s: %w", t.Src, err)
					}
					break
				}
			}
			if owner == nil {
				return nil, fmt.Errorf("%s: schema needs its owner's declaration", t.Src)
			}
			if b.owners == nil {
				b.owners = map[string]*Package{}
			}
			b.owners[t.Qualifier] = owner
		}
		if named := owner.named[t.Name]; named != nil {
			child := &schemer{p: owner, owners: b.owners, pending: map[string]bool{}, foreignPending: b.foreignPending}
			return child.value(named, append(slices.Clone(named.Rules), rules...), description)
		}
		foreign, err := owner.schemaWithOwners(t.Name, b.owners, b.foreignPending)
		if err != nil {
			return nil, err
		}
		for _, member := range foreign {
			if member.key == "$defs" {
				return nil, fmt.Errorf("%s: recursive foreign schema is unsupported", t.Src)
			}
		}
		return foreign, nil
	}
	if description == "" {
		description = b.describe(t)
	}
	var schema schemaObject
	switch t.Kind {
	case KindRaw:
		// Any JSON value: the schema that allows everything.
		return true, nil
	case KindStruct, KindUnion:
		return b.reference(t, description)
	case KindString:
		schema = stringSchema(b.p, rules)
	case KindBool:
		schema.add("type", "boolean")
	case KindInt, KindUint:
		schema = integerSchema(t, rules)
	case KindFloat:
		schema = floatSchema(rules)
	case KindSlice:
		items, err := b.value(t.Elem, innerRules(rules, RuleEach), "")
		if err != nil {
			return nil, err
		}
		schema.add("type", "array")
		schema.add("items", items)
		for _, rule := range rules {
			switch rule.Kind {
			case RuleItems:
				schema = bound(schema, rule, "minItems", "maxItems")
			case RuleUnique:
				schema.add("uniqueItems", true)
			}
		}
	case KindMap:
		values, err := b.value(t.Elem, innerRules(rules, RuleEach), "")
		if err != nil {
			return nil, err
		}
		if t.Elem.Kind == KindPointer {
			if object, ok := values.(schemaObject); ok {
				values = nullable(object)
			} else if values != true {
				return nil, fmt.Errorf("nullable map value %s has no object schema", t.Elem.Src)
			}
		}
		schema.add("type", "object")
		schema.add("additionalProperties", values)
		for _, rule := range rules {
			if rule.Kind == RuleItems {
				schema = bound(schema, rule, "minProperties", "maxProperties")
			}
		}
	default:
		return nil, fmt.Errorf("no schema for %s", t.Src)
	}
	if description != "" {
		schema.add("description", description)
	}
	return schema, nil
}

// describe returns the description of a type: a value's, a struct's or a
// union's.
func (b *schemer) describe(t *Type) string {
	switch t.Kind {
	case KindStruct:
		if s, ok := b.p.Structs[t.Name]; ok {
			return s.Description
		}
	case KindUnion:
		return b.p.Unions[t.Name].Description
	default:
		return t.Description
	}
	return ""
}

// reference returns the schema of a struct or a union, inline; a struct that is
// being expanded is a reference to its definition.
func (b *schemer) reference(t *Type, description string) (any, error) {
	if t.Opaque {
		return nil, fmt.Errorf("%s is decoded by its package and has no schema", t.Name)
	}
	var schema schemaObject
	var err error
	if t.Kind == KindUnion {
		schema, err = b.union(b.p.Unions[t.Name])
	} else {
		if b.pending[t.Name] {
			if !slices.Contains(b.refs, t.Name) {
				b.refs = append(b.refs, t.Name)
			}
			return schemaObject{{"$ref", "#/$defs/" + t.Name}}, nil
		}
		schema, err = b.structure(b.p.Structs[t.Name])
	}
	if err != nil {
		return nil, err
	}
	if description != "" {
		schema.add("description", description)
	}
	return schema, nil
}

// union returns the schema of a union: one of its variants, told apart by its
// tag, or any of them.
func (b *schemer) union(u *Union) (schemaObject, error) {
	if u.ContentName != "" {
		return nil, fmt.Errorf("%s: an adjacently tagged union has no JSON Schema", u.Name)
	}
	var variants []any
	for _, v := range u.Variants {
		var schema any
		var err error
		if v.Scalar != nil {
			schema, err = b.value(v.Scalar, nil, v.Description)
		} else {
			var object schemaObject
			object, err = b.structure(v)
			if err == nil && v.Description != "" {
				object.add("description", v.Description)
			}
			schema = object
		}
		if err != nil {
			return nil, fmt.Errorf("%s: %w", v.Name, err)
		}
		variants = append(variants, schema)
	}
	keyword := "anyOf"
	if u.TagName != "" {
		keyword = "oneOf"
	}
	return schemaObject{{keyword, variants}}, nil
}

// bound adds the lower and the upper limit of a rule.
func bound(schema schemaObject, rule Rule, lower, upper string) schemaObject {
	if rule.Min != nil {
		schema.add(lower, rule.Min.Value)
	}
	if rule.Max != nil {
		schema.add(upper, rule.Max.Value)
	}
	return schema
}

func stringSchema(p *Package, rules []Rule) schemaObject {
	var schema schemaObject
	schema.add("type", "string")
	for _, rule := range rules {
		switch rule.Kind {
		case RuleOneOf:
			values := make([]any, len(rule.Values))
			for i, value := range rule.Values {
				values[i] = value
			}
			schema.add("enum", values)
		case RuleEq:
			schema.add("const", rule.Value)
		case RuleChars:
			schema = bound(schema, rule, "minLength", "maxLength")
		case RulePattern:
			schema.add("pattern", p.patterns[rule.Value])
		}
	}
	return schema
}

// integerSchema returns the schema of an integer: its format is its Go type's,
// and an unsigned integer is at least 0 unless a rule says more.
func integerSchema(t *Type, rules []Rule) schemaObject {
	var schema schemaObject
	schema.add("type", "integer")
	format := "int"
	if t.Kind == KindUint {
		format = "uint"
	}
	if t.Bits != 0 {
		format += strconv.Itoa(t.Bits)
	}
	schema.add("format", format)
	var minimum, maximum *Bound
	for _, rule := range rules {
		if rule.Kind == RuleRange {
			minimum, maximum = rule.Min, rule.Max
		}
	}
	switch {
	case minimum != nil:
		schema.add("minimum", minimum.Value)
	case t.Kind == KindUint:
		schema.add("minimum", 0)
	}
	if maximum != nil {
		schema.add("maximum", maximum.Value)
	}
	return schema
}

func floatSchema(rules []Rule) schemaObject {
	var schema schemaObject
	schema.add("type", "number")
	schema.add("format", "double")
	for _, rule := range rules {
		if rule.Kind != RuleRange {
			continue
		}
		if rule.Min != nil {
			schema.add("minimum", rule.Min.Float)
		}
		if rule.Max != nil {
			schema.add("maximum", rule.Max.Float)
		}
	}
	return schema
}

// BrowserScalarSchema adds bounds and naming to the native opaque string format.
// Both browser emitters consume the same value;
// Inline determines whether fields embed the schema or reference a definition.
type BrowserScalarSchema struct {
	Type      string  `json:"type"`
	Format    string  `json:"format,omitzero"`
	Pattern   string  `json:"pattern,omitzero"`
	MinLength *uint64 `json:"minLength,omitzero"`
	MaxLength *uint64 `json:"maxLength,omitzero"`
	Inline    bool    `json:"-"`
}

// JSONSchema emits the browser scalar's JSON Schema without losing its format.
func (s BrowserScalarSchema) JSONSchema() ([]byte, error) {
	if err := s.check(); err != nil {
		return nil, err
	}
	return json.Marshal(s)
}
func (s BrowserScalarSchema) check() error {
	if s.Type != "string" {
		return fmt.Errorf("browser opaque scalar: unsupported type %q", s.Type)
	}
	switch s.Format {
	case "", "email", "trimmed", "http-url", "date-time":
	default:
		return fmt.Errorf("browser opaque scalar: unsupported format %q", s.Format)
	}
	if s.MinLength != nil && s.MaxLength != nil && *s.MinLength > *s.MaxLength {
		return fmt.Errorf("browser opaque scalar: minimum length exceeds maximum")
	}
	return nil
}
