package main

import (
	"encoding/json"
	"fmt"
	"go/types"
	"math"
	"math/big"
	"reflect"
	"slices"
	"strconv"
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
		value, err := g.jsonSchema(d.typ, nil, map[string]bool{})
		if err != nil {
			return nil, fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
		data, err := json.Marshal(value)
		if err != nil {
			return nil, fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
		out[key] = data
	}
	return out, nil
}

// jsonSchema follows the codec's contract graph, inlining each occurrence.
func (g *generator) jsonSchema(t types.Type, marks map[string]string, active map[string]bool) (map[string]any, error) {
	s := map[string]any{}
	if isJSON(t) {
		return s, nil
	}
	if p, ok := t.(*types.Pointer); ok {
		return g.jsonSchema(p.Elem(), marks, active)
	}
	switch t := t.(type) {
	case *types.Named:
		key := typeKey(t)
		d := g.defs[key]
		if active[key] {
			return nil, fmt.Errorf("%s: recursive schemas cannot be inlined", d.name)
		}
		active[key] = true
		defer delete(active, key)
		for _, marker := range []string{"check", "format", "timestamp", "base64"} {
			if has(d.marks, marker) {
				return nil, fmt.Errorf("%s: %s cannot be expressed faithfully in JSON Schema", d.name, marker)
			}
		}
		if has(d.marks, "union") {
			branches := []any{}
			for _, v := range g.variants(key) {
				child, err := g.jsonSchema(v.typ, nil, active)
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
					if _, err := g.jsonSchema(f.Type(), nil, active); err != nil {
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
				child, err := g.jsonSchema(f.Type(), m, active)
				if err != nil {
					return nil, fmt.Errorf("%s.%s: %w", d.name, f.Name(), err)
				}
				if has(m, "nullable") {
					child = map[string]any{"anyOf": []any{child, map[string]any{"type": "null"}}}
				}
				properties[opts[0]] = child
				if len(opts) == 1 {
					required = append(required, opts[0])
				}
			}
			s = map[string]any{"type": "object", "properties": properties}
			if !has(d.marks, "tolerant") {
				s["additionalProperties"] = false
			}
			if len(required) > 0 {
				s["required"] = required
			}
		} else {
			var err error
			s, err = g.jsonSchema(t.Underlying(), nil, active)
			if err != nil {
				return nil, err
			}
		}
		if err := schemaRules(s, d.marks); err != nil {
			return nil, err
		}
	case *types.Basic:
		switch {
		case t.Info()&types.IsString != 0:
			s["type"] = "string"
		case t.Info()&types.IsBoolean != 0:
			s["type"] = "boolean"
		case t.Info()&types.IsInteger != 0:
			s["type"] = "integer"
			bits := uint(64)
			switch t.Kind() {
			case types.Int8, types.Uint8:
				bits = 8
			case types.Int16, types.Uint16:
				bits = 16
			case types.Int32, types.Uint32:
				bits = 32
			}
			minimum := new(big.Int)
			maximum := new(big.Int)
			if t.Info()&types.IsUnsigned != 0 {
				maximum.Sub(new(big.Int).Lsh(big.NewInt(1), bits), big.NewInt(1))
			} else {
				limit := new(big.Int).Lsh(big.NewInt(1), bits-1)
				minimum.Neg(limit)
				maximum.Sub(limit, big.NewInt(1))
			}
			s["minimum"] = json.Number(minimum.String())
			s["maximum"] = json.Number(maximum.String())
		case t.Info()&types.IsFloat != 0:
			s["type"] = "number"
			limit := math.MaxFloat64
			if t.Kind() == types.Float32 {
				limit = math.MaxFloat32
			}
			s["minimum"] = json.Number(strconv.FormatFloat(-limit, 'g', -1, 64))
			s["maximum"] = json.Number(strconv.FormatFloat(limit, 'g', -1, 64))
		}
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return nil, fmt.Errorf("byte encoding cannot be expressed faithfully in JSON Schema")
		}
		child, err := g.jsonSchema(t.Elem(), nil, active)
		if err != nil {
			return nil, err
		}
		s = map[string]any{"type": "array", "items": child}
	case *types.Map:
		child, err := g.jsonSchema(t.Elem(), nil, active)
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
	for _, marker := range []string{"timestamp", "base64"} {
		if has(marks, marker) {
			return fmt.Errorf("%s cannot be expressed faithfully in JSON Schema", marker)
		}
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
