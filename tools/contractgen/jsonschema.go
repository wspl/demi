package main

import (
	"cmp"
	"encoding/json"
	"fmt"
	"go/types"
	"maps"
	"math/big"
	"reflect"
	"slices"
	"strings"
)

// jsonSchemas derives command schemas before either output is written.
func (g *generator) jsonSchemas() (map[string][]byte, error) {
	out := map[string][]byte{}
	for _, key := range g.order {
		d := g.defs[key]
		if !has(d.marks, "schema") {
			continue
		}
		emitter := schemaEmitter{g: g, root: key, active: map[string]bool{}, definitions: map[string]any{}, names: map[string]string{}}
		value, err := emitter.schema(d.typ, nil)
		if err != nil {
			return nil, fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
		object := value.(map[string]any)
		object["title"] = d.name
		if len(emitter.definitions) > 0 {
			object["$defs"] = emitter.definitions
		}
		data, err := json.Marshal(object)
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
	definitions map[string]any
	names       map[string]string
}

// schema follows the codec's contract graph, inlining nonrecursive occurrences.
func (e *schemaEmitter) schema(t types.Type, marks map[string]string) (any, error) {
	g := e.g
	s := map[string]any{}
	if isJSON(t) {
		return true, nil
	}
	if p, ok := t.(*types.Pointer); ok {
		return e.schema(p.Elem(), marks)
	}
	switch t := t.(type) {
	case *types.Named:
		key := typeKey(t)
		d := g.defs[key]
		if e.active[key] {
			if key == e.root {
				return map[string]any{"$ref": "#"}, nil
			}
			name := e.names[key]
			if name == "" {
				name = d.name
				for i := 2; slices.Contains(slices.Collect(maps.Values(e.names)), name); i++ {
					name = fmt.Sprintf("%s%d", d.name, i)
				}
				e.names[key] = name
				e.definitions[name] = nil
			}
			return map[string]any{"$ref": "#/$defs/" + name}, nil
		}
		e.active[key] = true
		defer delete(e.active, key)
		for _, marker := range []string{"check", "format", "base64"} {
			if has(d.marks, marker) {
				return nil, fmt.Errorf("%s: %s cannot be expressed faithfully in JSON Schema", d.name, marker)
			}
		}
		if has(d.marks, "union") {
			branches := []any{}
			variants := g.variants(key)
			slices.SortFunc(variants, func(a, b *definition) int { return cmp.Compare(a.typ.Obj().Pos(), b.typ.Obj().Pos()) })
			for _, v := range variants {
				child, err := e.schema(v.typ, nil)
				if err != nil {
					return nil, err
				}
				branches = append(branches, child)
			}
			s["oneOf"] = branches
		} else if st, ok := g.object(d); ok {
			// Flattening changes properties, but embedded custom checks still run
			// in the decoder and must not disappear from schema eligibility.
			original := d.typ.Underlying().(*types.Struct)
			for i := 0; i < original.NumFields(); i++ {
				f := original.Field(i)
				if f.Embedded() {
					if _, err := e.schema(f.Type(), nil); err != nil {
						return nil, fmt.Errorf("%s.%s: %w", d.name, f.Name(), err)
					}
				}
			}
			properties := map[string]any{}
			required := []string{}
			if union, tag, ok := strings.Cut(d.marks["variant"], " "); ok {
				name := bounds(g.defs[union].marks["union"])["tag"]
				properties[name] = map[string]any{"type": "string", "const": tag}
				required = append(required, name)
			}
			for i := 0; i < st.NumFields(); i++ {
				f := st.Field(i)
				opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
				m := d.fields[f.Name()]
				child, err := e.schema(f.Type(), m)
				if err != nil {
					return nil, fmt.Errorf("%s.%s: %w", d.name, f.Name(), err)
				}
				if has(m, "nullable") {
					child = map[string]any{"anyOf": []any{child, map[string]any{"type": "null"}}}
				}
				if description := d.fieldDescriptions[f.Name()]; description != "" {
					if object, ok := child.(map[string]any); ok {
						object["description"] = description
					} else {
						child = map[string]any{"description": description}
					}
				}
				properties[opts[0]] = child
				if len(opts) == 1 {
					required = append(required, opts[0])
				}
			}
			s = map[string]any{"type": "object"}
			if len(properties) > 0 {
				s["properties"] = properties
			}
			if !has(d.marks, "tolerant") {
				s["additionalProperties"] = false
			}
			if len(required) > 0 {
				s["required"] = required
			}
		} else {
			value, err := e.schema(t.Underlying(), nil)
			if err != nil {
				return nil, err
			}
			if object, ok := value.(map[string]any); ok {
				s = object
			}
		}
		if err := schemaRules(s, d.marks); err != nil {
			return nil, err
		}
		if d.description != "" {
			s["description"] = d.description
		}
		if name := e.names[key]; name != "" {
			e.definitions[name] = maps.Clone(s)
		}
	case *types.Basic:
		switch {
		case t.Info()&types.IsString != 0:
			s["type"] = "string"
		case t.Info()&types.IsBoolean != 0:
			s["type"] = "boolean"
		case t.Info()&types.IsInteger != 0:
			s["type"] = "integer"
			s["format"] = types.Typ[t.Kind()].Name()
			switch t.Kind() {
			case types.Int8:
				s["minimum"] = json.Number("-128")
				s["maximum"] = json.Number("127")
			case types.Int16:
				s["minimum"] = json.Number("-32768")
				s["maximum"] = json.Number("32767")
			case types.Uint8:
				s["minimum"] = json.Number("0")
				s["maximum"] = json.Number("255")
			case types.Uint16:
				s["minimum"] = json.Number("0")
				s["maximum"] = json.Number("65535")
			default:
				if t.Info()&types.IsUnsigned != 0 {
					s["minimum"] = json.Number("0")
				}
			}
		case t.Info()&types.IsFloat != 0:
			s["type"] = "number"
			s["format"] = "double"
			if t.Kind() == types.Float32 {
				s["format"] = "float"
			}
		}
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return nil, fmt.Errorf("byte encoding cannot be expressed faithfully in JSON Schema")
		}
		child, err := e.schema(t.Elem(), nil)
		if err != nil {
			return nil, err
		}
		s = map[string]any{"type": "array", "items": child}
	case *types.Map:
		child, err := e.schema(t.Elem(), nil)
		if err != nil {
			return nil, err
		}
		if isPointer(t.Elem()) {
			child = map[string]any{"anyOf": []any{child, map[string]any{"type": "null"}}}
		}
		s = map[string]any{"type": "object", "additionalProperties": child}
	default:
		return nil, fmt.Errorf("unsupported JSON Schema shape %s", t)
	}
	if err := schemaRules(s, marks); err != nil {
		return nil, err
	}
	return s, nil
}

// schemaRules intersects field constraints with the named contract's rules.
func schemaRules(s map[string]any, marks map[string]string) error {
	if has(marks, "base64") {
		return fmt.Errorf("base64 cannot be expressed faithfully in JSON Schema")
	}
	if has(marks, "timestamp") {
		s["format"] = "date-time"
	}
	if values := marks["enum"]; values != "" {
		next := strings.Fields(values)
		if previous, ok := s["enum"].([]string); ok {
			next = slices.DeleteFunc(next, func(value string) bool { return !slices.Contains(previous, value) })
		}
		if len(next) == 0 {
			return fmt.Errorf("enum constraints have no common value")
		}
		s["enum"] = next
	}
	if pattern := marks["pattern"]; pattern != "" {
		if previous, ok := s["pattern"]; ok && previous != pattern {
			s["allOf"] = []any{map[string]any{"pattern": previous}, map[string]any{"pattern": pattern}}
			delete(s, "pattern")
		} else {
			s["pattern"] = pattern
		}
	}
	for _, rule := range []string{"range", "length"} {
		for bound, value := range bounds(marks[rule]) {
			keyword := "minimum"
			if bound == "max" {
				keyword = "maximum"
			}
			if rule == "length" {
				keyword = "minLength"
				if bound == "max" {
					keyword = "maxLength"
				}
				if s["type"] == "array" {
					keyword = strings.Replace(keyword, "Length", "Items", 1)
				}
			}
			if old, ok := s[keyword].(json.Number); ok {
				// Marker bounds were checked before schema emission; generated
				// representation bounds are valid numeric literals too.
				a, _ := new(big.Rat).SetString(old.String())
				b, _ := new(big.Rat).SetString(value)
				if bound == "min" && a.Cmp(b) > 0 || bound == "max" && a.Cmp(b) < 0 {
					continue
				}
			}
			s[keyword] = json.Number(value)
		}
	}
	return nil
}
