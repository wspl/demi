package wiregen

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"go/ast"
	"go/token"
	"strings"
)

// opaqueRepresentation records schema metadata without generating an opaque codec.
func (p *reader) opaqueRepresentation(m marked, arguments []string) error {
	t := &Type{Kind: KindStruct, Name: m.name, Src: m.name, Opaque: true}
	if len(arguments) > 0 {
		switch arguments[0] {
		case "string":
			t.WireKind = KindString
		case "number":
			t.WireKind = KindFloat
		case "boolean":
			t.WireKind = KindBool
		default:
			return fmt.Errorf("%s: opaque representation is string, number or boolean", m.name)
		}
		seen := map[string]bool{}
		for _, argument := range arguments[1:] {
			key, value, ok := strings.Cut(argument, "=")
			if !ok || value == "" || seen[key] {
				return fmt.Errorf("%s: repeated or empty opaque metadata", m.name)
			}
			seen[key] = true
			if key == "format" {
				if t.WireKind != KindString {
					return fmt.Errorf("%s: format requires a string", m.name)
				}
				t.Format = value
				continue
			}
			if key != "chars" && key != "bytes" && key != "range" {
				return fmt.Errorf("%s: unsupported opaque bound %s", m.name, key)
			}
			rules, err := p.parseRules(argument)
			if err != nil {
				return err
			}
			if err := p.checkRules(&Type{Kind: t.WireKind}, rules); err != nil {
				return err
			}
			for _, rule := range rules {
				if rule.Min == nil || rule.Max == nil {
					continue
				}
				min, max := float64(rule.Min.Value), float64(rule.Max.Value)
				if rule.Min.Fraction {
					min = rule.Min.Float
				}
				if rule.Max.Fraction {
					max = rule.Max.Float
				}
				if min > max {
					return fmt.Errorf("%s: opaque lower bound exceeds upper bound", m.name)
				}
			}

			t.SchemaRules = append(t.SchemaRules, rules...)
		}
	}
	if lines := m.lines("//demi:jsonschema"); len(lines) > 0 {
		if len(lines) != 1 {
			return fmt.Errorf("%s: one jsonschema declaration is allowed", m.name)
		}
		mode, value, ok := strings.Cut(lines[0], " ")
		if !ok {
			return fmt.Errorf("%s: jsonschema requires inline, named or ref and a schema", m.name)
		}
		switch mode {
		case "ref":
			if !token.IsIdentifier(value) {
				return fmt.Errorf("%s: schema reference must name a declaration", m.name)
			}
			t.SchemaRef = value
		case "inline", "named":
			var schema map[string]jsontext.Value
			if err := json.Unmarshal([]byte(value), &schema); err != nil || schema == nil {
				return fmt.Errorf("%s: schema must be a JSON object", m.name)
			}
			t.Schema = []byte(value)
			t.SchemaInline = mode == "inline"
		default:
			return fmt.Errorf("%s: jsonschema requires inline, named or ref", m.name)
		}
	}
	if len(arguments) > 0 || len(t.Schema) > 0 || t.SchemaRef != "" || m.has("//demi:representation") {
		p.pkg.OpaqueScalars[m.name] = t
	}
	return nil
}

// resolveRepresentations reads an opaque storage field as its wire schema.
func (p *reader) resolveRepresentations() error {
	for _, m := range p.opaqueDecls {
		lines := m.lines("//demi:representation")
		if len(lines) == 0 {
			continue
		}
		if len(lines) != 1 || !token.IsIdentifier(lines[0]) {
			return fmt.Errorf("%s: representation names one storage field", m.name)
		}
		t := p.pkg.OpaqueScalars[m.name]
		if t.WireKind != 0 || len(t.Schema) > 0 || t.SchemaRef != "" {
			return fmt.Errorf("%s: representation cannot also declare a scalar or schema", m.name)
		}
		structure := m.spec.Type.(*ast.StructType)
		p.file = m.file
		for _, field := range structure.Fields.List {
			if len(field.Names) != 1 || field.Names[0].Name != lines[0] {
				continue
			}
			f, err := p.field(m.name, field)
			if err != nil {
				return err
			}
			t.Representation = &Type{}
			*t.Representation = *f.Type
			t.Representation.Rules = f.Rules
			t.WireKind = f.Type.Kind
		}
		if t.Representation == nil {
			return fmt.Errorf("%s: representation field %s is absent", m.name, lines[0])
		}
	}
	return nil
}

// namedSchema attaches explicit schema metadata without changing scalar decoding.
func (p *reader) namedSchema(m marked, named *Type) error {
	if !m.has("//demi:jsonschema") {
		return nil
	}
	if err := p.opaqueRepresentation(m, nil); err != nil {
		return err
	}
	schema := p.pkg.OpaqueScalars[m.name]
	named.Schema = schema.Schema
	named.SchemaInline = schema.SchemaInline
	named.SchemaRef = schema.SchemaRef
	named.WireKind = named.Kind
	p.pkg.OpaqueScalars[m.name] = named
	return nil
}
