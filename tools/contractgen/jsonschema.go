package main

import (
	"cmp"
	"encoding/json"
	"fmt"
	"go/types"
	"math/big"
	"reflect"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// jsonSchemas derives the selected schema use before either output is written.
func (g *generator) jsonSchemas(references bool) (map[string][]byte, error) {
	out := map[string][]byte{}
	for _, key := range g.order {
		d := g.defs[key]
		if !has(d.marks, "schema") {
			continue
		}
		emitter := schemaEmitter{
			g:           g,
			root:        key,
			references:  references,
			active:      map[string]bool{},
			definitions: &schemaObject{},
			names:       map[string]string{},
		}
		value, err := emitter.schema(d.typ)
		if err != nil {
			return nil, fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
		object := value.(*schemaObject)
		object.set("title", d.name)
		if len(emitter.definitions.fields) > 0 {
			object.set("$defs", emitter.definitions)
		}
		// A command schema's root keywords are sorted by name; nested objects keep
		// their insertion order.
		slices.SortFunc(object.fields, func(a, b contract.Field) int {
			return cmp.Compare(a.Name, b.Name)
		})
		data, err := contract.EncodeJSON(object)
		if err != nil {
			return nil, fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
		out[key] = data
	}
	return out, nil
}

type schemaEmitter struct {
	g           *generator
	root        string
	active      map[string]bool
	definitions *schemaObject
	references  bool
	names       map[string]string
}

// schema follows the codec's contract graph, inlining nonrecursive occurrences.
func (e *schemaEmitter) schema(t types.Type) (any, error) {
	s := &schemaObject{}
	if isJSON(t) {
		return true, nil
	}
	if p, ok := t.(*types.Pointer); ok {
		return e.schema(p.Elem())
	}
	switch t := t.(type) {
	case *types.Named:
		return e.namedSchema(t)
	case *types.Basic:
		basicSchema(t, s)
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return nil, fmt.Errorf("byte encoding cannot be expressed faithfully in JSON Schema")
		}
		child, err := e.subschema(t.Elem())
		if err != nil {
			return nil, err
		}
		s = schemaKeywords(
			contract.Field{Name: "type", Value: "array"},
			contract.Field{Name: "items", Value: serializedSchema(child, false)},
		)
	case *types.Map:
		child, err := e.subschema(t.Elem())
		if err != nil {
			return nil, err
		}
		if isPointer(t.Elem()) {
			child = nullableSchema(child)
		}
		s = schemaKeywords(
			contract.Field{Name: "type", Value: "object"},
			contract.Field{Name: "additionalProperties", Value: child},
		)
	default:
		return nil, fmt.Errorf("unsupported JSON Schema shape %s", t)
	}
	return s, nil
}

// schemaRules intersects field constraints with the named contract's rules.
func schemaRules(s *schemaObject, marks map[string]string) error {
	var instanceTypes []string
	switch typ := s.get("type").(type) {
	case string:
		instanceTypes = []string{typ}
	case []string:
		instanceTypes = typ
	}

	if has(marks, "base64") {
		return fmt.Errorf("base64 cannot be expressed faithfully in JSON Schema")
	}
	if has(marks, "timestamp") && slices.Contains(instanceTypes, "string") {
		s.set("format", "date-time")
	}
	if values := marks["enum"]; values != "" {
		next := strings.Fields(values)
		if previous, ok := s.get("enum").([]string); ok {
			next = slices.DeleteFunc(next, func(value string) bool {
				return !slices.Contains(previous, value)
			})
		}
		if len(next) == 0 {
			return fmt.Errorf("enum constraints have no common value")
		}
		s.set("enum", next)
	}

	schemaBounds(s, marks, instanceTypes)
	if pattern := marks["pattern"]; pattern != "" {
		if previous := s.get("pattern"); previous != nil && previous != pattern {
			s.set(
				"allOf",
				[]any{
					schemaKeywords(contract.Field{Name: "pattern", Value: previous}),
					schemaKeywords(contract.Field{Name: "pattern", Value: pattern}),
				},
			)
			s.remove("pattern")
		} else {
			s.set("pattern", pattern)
		}
	}
	if format := marks["format"]; format != "" {
		s.set("format", format)
	}
	return nil
}

// nullableSchema admits null: a typed schema adds "null" to its type and null to its
// const or enum; a schema with if, allOf, anyOf, oneOf or $ref becomes anyOf it or {"type":"null"}.
func nullableSchema(value any) any {
	s, ok := value.(*schemaObject)
	if !ok {
		if value == false {
			return schemaKeywords(contract.Field{Name: "type", Value: "null"})
		}
		return value
	}
	for _, key := range []string{"if", "allOf", "anyOf", "oneOf", "$ref"} {
		if s.get(key) != nil {
			return schemaKeywords(
				contract.Field{
					Name:  "anyOf",
					Value: []any{s, schemaKeywords(contract.Field{Name: "type", Value: "null"})},
				},
			)
		}
	}
	if typ, ok := s.get("type").(string); ok && typ != "null" {
		s.set("type", []string{typ, "null"})
	}
	if v := s.get("const"); v != nil {
		s.remove("const")
		s.set("enum", []any{v, nil})
	} else if values, ok := s.get("enum").([]string); ok {
		nullable := make([]any, 0, len(values)+1)
		for _, v := range values {
			nullable = append(nullable, v)
		}
		s.set("enum", append(nullable, nil))
	}
	return s
}

func (e *schemaEmitter) namedSchema(t *types.Named) (any, error) {
	s := &schemaObject{}
	key := typeKey(t)
	d := e.g.defs[key]
	if has(d.marks, "codec") && d.marks["codec"] != "string" {
		return nil, fmt.Errorf("%s: codec has no explicit JSON Schema mapping", d.name)
	}
	if e.active[key] {
		if key == e.root {
			return schemaKeywords(contract.Field{Name: "$ref", Value: "#"}), nil
		}
		name := e.definitionName(d)
		return schemaKeywords(contract.Field{Name: "$ref", Value: "#/$defs/" + name}), nil
	}
	e.active[key] = true
	defer delete(e.active, key)
	for _, marker := range []string{"format", "base64"} {
		if has(d.marks, marker) && d.marks["codec"] != "string" {
			return nil, fmt.Errorf("%s: %s cannot be expressed faithfully in JSON Schema", d.name, marker)
		}
	}
	var err error
	s, err = e.namedShape(t, d, key, s)
	if err != nil {
		return nil, err
	}
	if values := d.marks["enum"]; values != "" {
		s.set("enum", strings.Fields(values))
	}
	if d.description != "" && !primitiveSchema(d) {
		s.set("description", d.description)
	}
	if err := schemaRules(s, d.marks); err != nil {
		return nil, err
	}

	if name := e.names[key]; name != "" {
		e.definitions.set(name, s.clone())
	}
	return s, nil
}

func (e *schemaEmitter) objectSchema(d *definition, st *types.Struct) (*schemaObject, error) {
	g := e.g
	var s *schemaObject
	// Flattening changes properties, but embedded declarations must still
	// be checked for unsupported schema rules.
	original := d.typ.Underlying().(*types.Struct)
	for i := 0; i < original.NumFields(); i++ {
		f := original.Field(i)
		if f.Embedded() {
			if _, err := e.schema(f.Type()); err != nil {
				return nil, fmt.Errorf("%s.%s: %w", d.name, f.Name(), err)
			}
		}
	}
	properties := &schemaObject{}
	required := []string{}
	constraints := []any{}
	if name, tag, kind := g.variantWire(d); name != "" {
		var value any = tag
		typ := "string"
		if kind == "bool" {
			typ = "boolean"
			value = tag == "true"
		}
		properties.set(
			name,
			schemaKeywords(
				contract.Field{Name: "type", Value: typ},
				contract.Field{Name: "const", Value: value},
			),
		)
		required = append(required, name)
	}
	if u := g.adjacentUnion(d); u != nil && st.NumFields() == 0 {
		content := bounds(u.marks["union"])["content"]
		properties.set(content, schemaKeywords(contract.Field{Name: "type", Value: "null"}))
		required = append(required, content)
	}
	for i := 0; i < st.NumFields(); i++ {
		if err := e.schemaField(d, st, i, properties, &required, &constraints); err != nil {
			return nil, err
		}
	}
	// An internal tag comes after the payload properties but first in required.
	// Adjacent tags are created before content.
	if name, _, _ := g.variantWire(d); name != "" && g.adjacentUnion(d) == nil {
		value := properties.get(name)
		properties.remove(name)
		properties.set(name, value)
	}
	s = schemaKeywords(contract.Field{Name: "type", Value: "object"})
	if !has(d.marks, "tolerant") {
		s.set("additionalProperties", false)
	}
	if len(constraints) > 0 {
		s.set("allOf", constraints)
	}
	if len(properties.fields) > 0 {
		s.set("properties", properties)
	}
	if len(required) > 0 {
		s.set("required", required)
	}
	return s, nil
}

func (e *schemaEmitter) schemaField(
	d *definition,
	st *types.Struct,
	i int,
	properties *schemaObject,
	required *[]string,
	constraints *[]any,
) error {
	g := e.g

	f := st.Field(i)
	if nested := g.optionalObject(f); nested != nil {
		return e.optionalProperties(nested, properties)
	}
	if u := g.flattenedUnion(d, f); u != nil {
		return e.flattenedProperties(f, u, properties, required, constraints)
	}
	opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
	m := d.fields[f.Name()]
	child, err := e.subschema(f.Type())
	if err != nil {
		return fmt.Errorf("%s.%s: %w", d.name, f.Name(), err)
	}
	if has(m, "object") {
		child = schemaKeywords(
			contract.Field{Name: "type", Value: "object"},
			contract.Field{Name: "additionalProperties", Value: true},
		)
	}
	if has(m, "nullable") {
		child = nullableSchema(child)
	}
	if description := d.fieldDescriptions[f.Name()]; description != "" {
		if object, ok := child.(*schemaObject); ok {
			object.set("description", description)
		} else {
			child = schemaKeywords(contract.Field{Name: "description", Value: description})
		}
	}
	if object, ok := child.(*schemaObject); ok {
		if err := schemaRules(object, m); err != nil {
			return err
		}
	}
	e.fieldDefaults(f, opts, m, child)

	if u := g.adjacentUnion(d); u != nil {
		opts[0] = bounds(u.marks["union"])["content"]
	}
	properties.set(opts[0], child)
	if len(opts) == 1 && !has(m, "default") {
		*required = append(*required, opts[0])
	}
	return nil
}

// optionalProperties contributes keys without making the embedded object’s fields required.
func (e *schemaEmitter) optionalProperties(nested *definition, properties *schemaObject) error {
	child, err := e.schema(nested.typ)
	if err != nil {
		return err
	}
	object, ok := child.(*schemaObject)
	if !ok || object.get("type") != "object" {
		return fmt.Errorf("recursive optional flattened object is unsupported")
	}
	if props, ok := object.get("properties").(*schemaObject); ok {
		for _, property := range props.fields {
			properties.set(property.Name, property.Value)
		}
	}
	return nil
}

func (e *schemaEmitter) flattenedProperties(
	f *types.Var,
	u *definition,
	properties *schemaObject,
	required *[]string,
	constraints *[]any,
) error {
	union, err := e.schema(f.Type())
	if err != nil {
		return err
	}
	// The parent owns strictness; each branch constrains only its
	// contributed keys so sibling fields remain legal.
	object, ok := union.(*schemaObject)
	if !ok {
		return fmt.Errorf("expected adjacent union schema")
	}
	branches, ok := object.get("oneOf").([]any)
	if !ok {
		return fmt.Errorf("recursive flattened union schema is unsupported")
	}
	for _, branch := range branches {
		variant, ok := branch.(*schemaObject)
		if !ok {
			return fmt.Errorf("expected adjacent variant schema")
		}
		variant.remove("additionalProperties")
	}
	*constraints = append(*constraints, object)
	for _, key := range []string{bounds(u.marks["union"])["tag"], bounds(u.marks["union"])["content"]} {
		properties.set(key, true)
		*required = append(*required, key)
	}
	return nil
}

// fieldDefaults writes a field's default: null for an optional numbered pointer, the empty value for +demi:default.
func (e *schemaEmitter) fieldDefaults(f *types.Var, opts []string, m map[string]string, child any) {
	g := e.g
	if pointer, ok := f.Type().(*types.Pointer); ok && len(opts) > 1 {
		numbered := has(m, "integer")
		if named, ok := pointer.Elem().(*types.Named); ok {
			numbered = numbered || has(g.defs[typeKey(named)].marks, "integer")
		}
		if numbered {
			// An optional numbered pointer is annotated "default": null,
			// even though the decoder refuses an explicit null.
			child.(*schemaObject).set("default", nil)
		}
	}
	if has(m, "default") {
		value := any(nil)
		switch underlying := f.Type().Underlying().(type) {
		case *types.Slice:
			value = []any{}
		case *types.Map:
			value = &schemaObject{}
		case *types.Basic:
			switch {
			case underlying.Info()&types.IsString != 0:
				value = ""
			case underlying.Info()&types.IsBoolean != 0:
				value = false
			default:
				value = 0
			}
		}
		child.(*schemaObject).set("default", value)
	}
}

func schemaBounds(s *schemaObject, marks map[string]string, instanceTypes []string) {
	for _, rule := range []string{"range", "length"} {
		for _, bound := range []string{"min", "max"} {
			value := bounds(marks[rule])[bound]
			if value == "" {
				continue
			}
			keyword := "minimum"
			if bound == "max" {
				keyword = "maximum"
			}
			if rule == "length" {
				keyword = "minLength"
				if bound == "max" {
					keyword = "maxLength"
				}
				if slices.Contains(instanceTypes, "array") {
					keyword = strings.Replace(keyword, "Length", "Items", 1)
				}
			}
			if old, ok := s.get(keyword).(json.Number); ok {
				// Marker bounds were checked before schema emission; generated
				// representation bounds are valid numeric literals too.
				a, _ := new(big.Rat).SetString(old.String())
				b, _ := new(big.Rat).SetString(value)
				if bound == "min" && a.Cmp(b) > 0 || bound == "max" && a.Cmp(b) < 0 {
					continue
				}
			}
			s.set(keyword, json.Number(value))
		}
	}
}

func basicSchema(t *types.Basic, s *schemaObject) {
	switch {
	case t.Info()&types.IsString != 0:
		s.set("type", "string")
	case t.Info()&types.IsBoolean != 0:
		s.set("type", "boolean")
	case t.Info()&types.IsInteger != 0:
		s.set("type", "integer")
		s.set("format", types.Typ[t.Kind()].Name())
		switch t.Kind() {
		case types.Int8:
			s.set("minimum", json.Number("-128"))
			s.set("maximum", json.Number("127"))
		case types.Int16:
			s.set("minimum", json.Number("-32768"))
			s.set("maximum", json.Number("32767"))
		case types.Uint8:
			s.set("minimum", json.Number("0"))
			s.set("maximum", json.Number("255"))
		case types.Uint16:
			s.set("minimum", json.Number("0"))
			s.set("maximum", json.Number("65535"))
		default:
			if t.Info()&types.IsUnsigned != 0 {
				s.set("minimum", json.Number("0"))
			}
		}
	case t.Info()&types.IsFloat != 0:
		s.set("type", "number")
		s.set("format", "double")
		if t.Kind() == types.Float32 {
			s.set("format", "float")
		}
	}
}

func (e *schemaEmitter) unionSchema(d *definition, key string, s *schemaObject) error {
	g := e.g
	branches := []any{}
	variants := g.variants(key)
	slices.SortFunc(
		variants,
		func(a, b *definition) int {
			return cmp.Compare(a.typ.Obj().Pos(), b.typ.Obj().Pos())
		},
	)
	for _, v := range variants {
		child, err := e.schema(v.typ)
		if err != nil {
			return err
		}
		branches = append(branches, child)
	}
	if d.marks["union"] == "untagged" {
		s.set("anyOf", branches)
	} else {
		s.set("oneOf", branches)
	}
	return nil
}

func (e *schemaEmitter) namedShape(t *types.Named, d *definition, key string, s *schemaObject) (*schemaObject, error) {
	if has(d.marks, "union") {
		if err := e.unionSchema(d, key, s); err != nil {
			return nil, err
		}
		return s, nil
	}
	if st, ok := e.g.object(d); ok {
		return e.objectSchema(d, st)
	}
	value, err := e.schema(t.Underlying())
	if err != nil {
		return nil, err
	}
	if object, ok := value.(*schemaObject); ok {
		s = object
	}
	return s, nil
}
