package main

import (
	"fmt"
	"go/types"
	"reflect"
	"strconv"
	"strings"
)

// markReceived propagates tolerant web input through every reachable definition.
func (g *generator) markReceived(name string) {
	if g.received[name] {
		return
	}
	g.received[name] = true
	d := g.defs[name]
	if d == nil {
		g.err = fmt.Errorf("external contract %s is not loaded", name)
		return
	}
	if has(d.marks, "union") {
		for _, v := range g.variants(name) {
			g.markReceived(v.key)
		}
	}
	var visit func(types.Type)
	visit = func(t types.Type) {
		if isJSON(t) {
			return
		}
		switch t := t.(type) {
		case *types.Named:
			g.markReceived(typeKey(t))
		case *types.Pointer:
			visit(t.Elem())
		case *types.Slice:
			visit(t.Elem())
		case *types.Map:
			visit(t.Elem())
		case *types.Struct:
			for i := 0; i < t.NumFields(); i++ {
				visit(t.Field(i).Type())
			}
		}
	}
	visit(d.typ.Underlying())
}

func (g *generator) emitTS(name string) {
	d := g.defs[name]
	if old, ok := g.tsNames[tsName(d.name)]; ok && old != name {
		g.err = fmt.Errorf("%s: %s: duplicate TypeScript name also from %s", d.position, d.name, old)
		return
	}
	g.tsNames[tsName(d.name)] = name
	if g.shared[name] && (has(d.marks, "root") || !has(d.marks, "variant") && !isPrivateScalar(d)) {
		g.tsImports[schema(d.name)] = true
		return
	}
	defer func() {
		if g.err != nil {
			g.err = fmt.Errorf("%s: %s: %w", d.position, d.name, g.err)
		}
	}()
	if g.emitted[name] || g.active[name] {
		return
	}
	g.active[name] = true
	if g.adjacentUnion(d) != nil {
		g.err = fmt.Errorf("adjacent union variant is not supported in TypeScript")
		return
	}
	var code string
	if has(d.marks, "union") {
		if bounds(d.marks["union"])["content"] != "" {
			g.err = fmt.Errorf("adjacent union is not supported in TypeScript")
			return
		}
		if d.marks["union"] == "untagged" {
			g.err = fmt.Errorf("untagged union is not supported in TypeScript")
			return
		}
		tag := bounds(d.marks["union"])["tag"]
		var variants []string
		for _, v := range g.variants(name) {
			g.emitTS(v.key)
			_, value, _ := strings.Cut(v.marks["variant"], " ")
			variants = append(variants, schema(v.name)+".extend({"+q(tag)+": z.literal("+tagLiteral(value)+")})")
		}
		code = "z.discriminatedUnion(" + q(tag) + ", [" + strings.Join(variants, ", ") + "])"
	} else if st, ok := g.object(d); ok {
		var fields []string
		for i := 0; i < st.NumFields(); i++ {
			f := st.Field(i)
			opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
			value, forward := g.tsType(f.Type(), d.fields[f.Name()])
			if has(d.fields[f.Name()], "nullable") {
				value += ".nullable()"
			}
			if len(opts) > 1 {
				value += ".optional()"
			}
			if forward {
				fields = append(fields, "get "+q(opts[0])+"() { return "+value+" }")
			} else {
				fields = append(fields, q(opts[0])+": "+value)
			}
		}
		constructor := "z.object"
		if !g.received[name] {
			constructor = "z.strictObject"
			if has(d.marks, "tolerant") {
				g.err = fmt.Errorf("%s: send-only object must refuse unknown fields", name)
			}
		}
		code = constructor + "({" + strings.Join(fields, ", ") + "})"
	} else {
		var forward bool
		code, forward = g.tsType(d.typ.Underlying(), d.marks)
		if forward {
			g.err = fmt.Errorf("%s: recursion outside an object property", name)
		}
	}
	export := "export "
	if !has(d.marks, "root") && (has(d.marks, "variant") || isPrivateScalar(d)) {
		export = ""
	}
	fmt.Fprintf(&g.ts, "%sconst %s = %s\n%stype %s = z.infer<typeof %s>\n", export, schema(d.name), code, export, tsName(d.name), schema(d.name))
	g.active[name] = false
	g.emitted[name] = true
}

func (g *generator) tsType(t types.Type, m map[string]string) (string, bool) {
	if isJSON(t) {
		return "z.json()", false
	}
	code := ""
	forward := false
	switch t := t.(type) {
	case *types.Pointer:
		return g.tsType(t.Elem(), m)
	case *types.Named:
		key := typeKey(t)
		forward = g.active[key]
		g.emitTS(key)
		code = schema(t.Obj().Name())
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			if has(m, "length") {
				g.err = fmt.Errorf("binary length rules are outside the supported Zod subset")
			}
			return base64Schema, false
		}
		item, f := g.tsType(t.Elem(), map[string]string{})
		code = "z.array(" + item + ")"
		forward = f
	case *types.Map:
		item, f := g.tsType(t.Elem(), map[string]string{})
		if isPointer(t.Elem()) {
			item += ".nullable()"
		}
		code = "z.record(z.string(), " + item + ")"
		forward = f
	case *types.Basic:
		switch t.Kind() {
		case types.String:
			code = "z.string()"
		case types.Bool:
			code = "z.boolean()"
		case types.Float32, types.Float64:
			code = "z.number()"
		case types.Int8:
			code = "z.int().min(-128).max(127)"
		case types.Uint8:
			code = "z.int().min(0).max(255)"
		case types.Int16:
			code = "z.int().min(-32768).max(32767)"
		case types.Uint16:
			code = "z.int().min(0).max(65535)"
		case types.Int32:
			code = "z.int().min(-2147483648).max(2147483647)"
		case types.Uint32:
			code = "z.int().min(0).max(4294967295)"
		case types.Int, types.Int64, types.Uint, types.Uint64:
			b := bounds(m["range"])
			maximum, err := strconv.ParseFloat(b["max"], 64)
			if err != nil || maximum > 9007199254740991 {
				g.err = fmt.Errorf("%s: integer read by JavaScript requires a safe maximum", t)
			}
			code = "z.int()"
			if t.Info()&types.IsUnsigned != 0 {
				code += ".min(0)"
			} else {
				minimum, err := strconv.ParseFloat(b["min"], 64)
				if err != nil || minimum < -9007199254740991 {
					g.err = fmt.Errorf("%s: integer read by JavaScript requires a safe minimum", t)
				}
			}
		default:
			g.err = fmt.Errorf("unsupported TypeScript scalar %s", t)
		}
	default:
		g.err = fmt.Errorf("unsupported TypeScript shape %s", t)
	}
	switch m["format"] {
	case "trimmed":
		code += ".trim()"
	case "email":
		code = "z.email().max(254)"
	case "http-url":
		code = "z.url({ protocol: z.regexes.httpProtocol })"
	}
	if has(m, "timestamp") && !integerTimestamp(t) {
		code = "z.iso.datetime({ precision: 3 })"
	}
	if has(m, "base64") {
		code = base64Schema
	}
	if values := m["enum"]; values != "" {
		var vals []string
		for _, v := range strings.Fields(values) {
			vals = append(vals, q(v))
		}
		code = "z.enum([" + strings.Join(vals, ", ") + "])"
	}
	for _, rule := range []string{"length", "range"} {
		b := bounds(m[rule])
		for _, key := range []string{"min", "max"} {
			if value := b[key]; value != "" && value != "9007199254740991" && value != "-9007199254740991" {
				code += "." + key + "(" + value + ")"
			}
		}
	}
	if pattern := m["pattern"]; pattern != "" {
		code += ".regex(new RegExp(" + q(pattern) + ", \"u\"))"
	}
	return code, forward
}

// Zod checks alphabet and padding length, but not unused padding bits. The
// additional pattern matches the canonical bytes accepted by Go's Strict codec.
const base64Schema = `z.base64().regex(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/][AQgw]==|[A-Za-z0-9+/]{2}[AEIMQUYcgkosw048]=)?$/)`
