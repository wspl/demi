package main

import (
	"fmt"
	"go/types"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

func isPointer(t types.Type) bool { _, ok := t.(*types.Pointer); return ok }

// check rejects contract declarations that cannot be represented faithfully.
func (g *generator) check(p *packages.Package) error {
	for _, name := range g.order {
		d := g.defs[name]
		fail := func(problem string) error {
			return fmt.Errorf("%s: %s: %s", p.Fset.Position(d.typ.Obj().Pos()), name, problem)
		}
		if err := checkMarks(d.marks); err != nil {
			return fail(err.Error())
		}
		if err := checkRuleType(d.typ, d.marks, false); err != nil {
			return fail(err.Error())
		}
		if err := checkCustom(d); err != nil {
			return fail(err.Error())
		}
		if _, pointer := d.typ.Underlying().(*types.Pointer); pointer {
			return fail("named pointer types cannot own generated methods")
		}
		if d.typ.TypeParams().Len() > 0 {
			return fail("generic declarations require a concrete named instantiation")
		}
		if has(d.marks, "strict") && has(d.marks, "tolerant") {
			return fail("strict and tolerant are mutually exclusive")
		}
		if has(d.marks, "union") {
			if _, err := unionSeal(d.typ); err != nil {
				return fail(err.Error())
			}
			if d.marks["msgpack"] == "tuple" && (d.marks["union"] == "untagged" || g.unionKind(d) == "bool") {
				return fail("tuple unions require string tags")
			}
			if bounds(d.marks["union"])["tag"] == "" && d.marks["union"] != "untagged" {
				return fail("union requires tag=<field>")
			}
			if content := bounds(d.marks["union"])["content"]; content != "" {
				if content == bounds(d.marks["union"])["tag"] || d.marks["msgpack"] == "tuple" {
					return fail("adjacent content requires a distinct key and map encoding")
				}
			}
			tags := map[string]bool{}
			for _, v := range g.variants(name) {
				_, tag, _ := strings.Cut(v.marks["variant"], " ")
				if tags[tag] && d.marks["union"] != "untagged" {
					return fail("duplicate variant tag " + tag)
				}
				if tag == "" && d.marks["union"] != "untagged" {
					return fail("tagged variant requires a tag")
				}
				tags[tag] = true
				if d.marks["union"] == "untagged" && has(v.marks, "tolerant") {
					return fail("untagged variants must be strict")
				}
				if _, _, kind := g.variantWire(v); d.marks["union"] != "untagged" && kind != g.unionKind(d) {
					return fail("union cannot mix boolean and string tags")
				}
			}
			if len(g.variants(name)) == 0 {
				return fail("union has no variants; use variant <Union> <tag>")
			}
		}
		if variant := d.marks["variant"]; variant != "" {
			parts := strings.Fields(variant)
			if len(parts) < 1 || len(parts) > 2 || g.defs[parts[0]] == nil || !has(g.defs[parts[0]].marks, "union") {
				return fail("variant requires a known union and tag")
			}
			if _, object := d.typ.Underlying().(*types.Struct); !object {
				basic, scalar := d.typ.Underlying().(*types.Basic)
				if g.defs[parts[0]].marks["union"] != "untagged" || !scalar || basic.Info()&(types.IsString|types.IsBoolean|types.IsInteger|types.IsFloat) == 0 {
					return fail("variant requires an object or an untagged scalar")
				}
			}
			if iface, ok := g.defs[parts[0]].typ.Underlying().(*types.Interface); ok && types.Implements(d.typ, iface) {
				return fail("variant sealing method must have a pointer receiver")
			}
		}
		if root, ok := d.marks["root"]; ok {
			args := bounds(root)
			if args["direction"] != "receive" && args["direction"] != "send" && (args["direction"] != "" || args["output"] != "") {
				return fail("root requires direction=receive|send")
			}
			if out := args["output"]; out != "" && out != "protocol" && out != "web" && !regexp.MustCompile(`^plugin-[a-z][a-z0-9-]*$`).MatchString(out) {
				return fail("invalid root output")
			}
		}
		if has(d.marks, "codec") {
			if err := checkCodec(d, g.msgReach[d.key]); err != nil {
				return fail(err.Error())
			}
			continue
		}
		if st, ok := g.object(d); ok {
			if g.err != nil {
				return g.err
			}
			if g.adjacentUnion(d) != nil && st.NumFields() > 1 {
				return fail("adjacent variants require zero fields or one content field")
			}
			seen := map[string]bool{}
			if union, _, ok := strings.Cut(d.marks["variant"], " "); ok {
				seen[bounds(g.defs[union].marks["union"])["tag"]] = true
			}
			for i := 0; i < st.NumFields(); i++ {
				f := st.Field(i)
				m := d.fields[f.Name()]
				if err := checkMarks(m); err != nil {
					return fail(f.Name() + ": " + err.Error())
				}
				if err := checkRuleType(f.Type(), m, true); err != nil {
					return fail(f.Name() + ": " + err.Error())
				}
				fieldType := f.Type()
				for {
					pointer, ok := fieldType.(*types.Pointer)
					if !ok {
						break
					}
					fieldType = pointer.Elem()
				}
				if named, ok := fieldType.(*types.Named); ok {
					if child := g.defs[typeKey(named)]; child != nil {
						if has(child.marks, "integer") && (has(m, "nullable") || has(m, "timestamp")) || has(m, "integer") && has(child.marks, "timestamp") {
							return fail(f.Name() + ": integer string cannot be nullable or a timestamp")
						}
					}
					if child := g.defs[typeKey(named)]; child != nil && has(child.marks, "codec") {
						for marker := range m {
							if marker != "nullable" {
								return fail(f.Name() + ": codec fields cannot have " + marker + " rules")
							}
						}
					}
				}
				tag := reflect.StructTag(st.Tag(i)).Get("json")
				if child := g.optionalObject(f); child != nil {
					if !f.Exported() || tag != "" || len(m) != 0 || g.adjacentUnion(d) != nil || g.tupleUnion(d) {
						return fail(f.Name() + ": optional embedded objects cannot have tags, rules, or tuple/adjacent content")
					}
					nested, _ := g.object(child)
					for _, key := range g.propertyNames(child, nested) {
						if seen[key] {
							return fail(f.Name() + ": duplicate flattened field " + key)
						}
						seen[key] = true
					}
					if g.err != nil {
						return g.err
					}
					continue
				}
				if has(m, "flatten") {
					u := g.flattenedUnion(d, f)
					if g.adjacentUnion(d) != nil || u == nil || !has(u.marks, "union") || bounds(u.marks["union"])["content"] == "" || tag != "" || !f.Exported() || len(m) != 1 {
						return fail(f.Name() + ": flatten requires an adjacent union field without a JSON tag or other rules")
					}
					for _, key := range []string{bounds(u.marks["union"])["tag"], bounds(u.marks["union"])["content"]} {
						if seen[key] {
							return fail(f.Name() + ": duplicate flattened field " + key)
						}
						seen[key] = true
					}
					continue
				}
				parts := strings.Split(tag, ",")
				if !f.Exported() || parts[0] == "" || parts[0] == "-" || seen[parts[0]] {
					return fail(f.Name() + ": requires a unique explicit JSON field name")
				}
				seen[parts[0]] = true
				for _, opt := range parts[1:] {
					if opt != "omitempty" && opt != "omitzero" {
						return fail(f.Name() + ": unsupported JSON option " + opt)
					}
				}
				if has(m, "default") && len(parts) > 1 {
					return fail(f.Name() + ": default fields must always be written")
				}
				if (len(parts) > 1 || has(m, "default")) && g.adjacentUnion(d) != nil {
					return fail(f.Name() + ": adjacent content cannot be optional")
				}
				if (len(parts) > 1 || has(m, "default")) && g.tupleUnion(d) {
					return fail(f.Name() + ": tuple fields cannot be optional")
				}
				if len(parts) > 2 {
					return fail(f.Name() + ": only one optionality option is allowed")
				}
				if len(parts) > 1 && !isPointer(f.Type()) && (parts[1] != "omitempty" || !emptyCollection(f.Type())) {
					b, ok := f.Type().Underlying().(*types.Basic)
					if !ok || b.Kind() != types.Bool {
						return fail(f.Name() + ": optional values use pointers, default-false booleans, or omitempty collections")
					}
				}
				if has(m, "nullable") && len(parts) > 1 && (parts[1] != "omitempty" || !isPointer(f.Type())) {
					return fail(f.Name() + ": optional nullable fields require a pointer with omitempty")
				}
				if err := g.checkType(f.Type()); err != nil {
					return fail(f.Name() + ": " + err.Error())
				}
			}
		} else if !has(d.marks, "union") {
			if err := g.checkType(d.typ.Underlying()); err != nil {
				return fail(err.Error())
			}
		}
	}
	return nil
}
func checkMarks(m map[string]string) error {
	if has(m, "format") && m["format"] != "email" && m["format"] != "http-url" && m["format"] != "trimmed" {
		return fmt.Errorf("format requires email, http-url or trimmed")
	}
	if has(m, "msgpack") && m["msgpack"] != "" && m["msgpack"] != "tuple" {
		return fmt.Errorf("msgpack accepts only tuple")
	}

	if problem := m["!error"]; problem != "" {
		return fmt.Errorf("%s", problem)
	}
	if value := m["codec"]; value != "" && value != "string" {
		return fmt.Errorf("codec accepts only string")
	}

	for _, key := range []string{"default", "nullable", "strict", "tolerant", "timestamp", "base64", "table", "schema", "flatten"} {
		if m[key] != "" {
			return fmt.Errorf("%s takes no arguments", key)
		}
	}
	for _, key := range []string{"union", "root"} {
		allowed := map[string]bool{"tag": true, "content": true}
		if key == "root" {
			allowed = map[string]bool{"direction": true, "output": true}
		}
		seen := map[string]bool{}
		for _, arg := range strings.Fields(m[key]) {
			if key == "union" && m[key] == "untagged" {
				continue
			}
			name, value, ok := strings.Cut(arg, "=")
			if !ok || !allowed[name] || value == "" || seen[name] {
				return fmt.Errorf("invalid %s argument %q", key, arg)
			}
			seen[name] = true
		}
	}
	for _, key := range []string{"length", "range"} {
		seen := map[string]bool{}
		for _, arg := range strings.Fields(m[key]) {
			if arg == "schema-only" && key == "range" && !seen[arg] {
				seen[arg] = true
				continue
			}
			if arg == "chars" && key == "length" {
				continue
			}
			name, value, ok := strings.Cut(arg, "=")
			if seen[name] {
				return fmt.Errorf("duplicate %s bound %s", key, name)
			}
			seen[name] = true
			if !ok || (name != "min" && name != "max") || !regexp.MustCompile(`^-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$`).MatchString(value) {
				return fmt.Errorf("invalid %s argument %q", key, arg)
			}
		}
	}
	pattern := m["pattern"]
	if has(m, "id") && m["id"] != "" {
		var ok bool
		pattern, ok = strings.CutPrefix(m["id"], "pattern=")
		if !ok || pattern == "" {
			return fmt.Errorf("id accepts only pattern=<regexp>")
		}
		if has(m, "pattern") {
			return fmt.Errorf("define the identifier pattern once")
		}
		m["pattern"] = pattern
	}
	if pattern != "" {
		if _, err := regexp.Compile(pattern); err != nil {
			return fmt.Errorf("pattern: %w", err)
		}
		if err := checkPattern(pattern); err != nil {
			return err
		}

	}
	return nil
}
func (g *generator) checkType(t types.Type) error {
	if isJSON(t) {
		return nil
	}
	switch t := t.(type) {
	case *types.Named:
		if t.TypeArgs().Len() > 0 {
			return fmt.Errorf("give the generic instantiation a concrete named declaration")
		}
		if g.defs[typeKey(t)] == nil {
			return fmt.Errorf("external contract %s is not loaded", t)
		}
		return nil
	case *types.Pointer:
		return g.checkType(t.Elem())
	case *types.Slice:
		return g.checkType(t.Elem())
	case *types.Map:
		if b, ok := t.Key().Underlying().(*types.Basic); !ok || b.Kind() != types.String {
			return fmt.Errorf("record keys must be strings")
		}
		if named, ok := t.Key().(*types.Named); ok {
			if d := g.defs[typeKey(named)]; d != nil && has(d.marks, "codec") {
				return fmt.Errorf("codec cannot be a record key: record keys use string encoding")
			}
		}
		if err := g.checkType(t.Key()); err != nil {
			return err
		}
		return g.checkType(t.Elem())
	case *types.Basic:
		if t.Kind() == types.Uintptr {
			return fmt.Errorf("uintptr is not a wire integer")
		}
		if t.Info()&(types.IsString|types.IsBoolean|types.IsInteger|types.IsFloat) != 0 {
			return nil
		}
	}
	return fmt.Errorf("unsupported shape %s", t)
}

// tsSources renders each output once and imports shared protocol schemas.
func (g *generator) tsSources() (map[string][]byte, error) {
	outputs := map[string][]string{}
	for _, key := range g.order {
		if root := g.defs[key].marks["root"]; root != "" {
			if out := bounds(root)["output"]; out != "" {
				outputs[out] = append(outputs[out], key)
			}
		}
	}
	g.received = map[string]bool{}
	for _, roots := range outputs {
		for _, key := range roots {
			if bounds(g.defs[key].marks["root"])["direction"] == "receive" {
				g.markReceived(key, true)
			}
		}
	}
	received := g.received
	// Protocol ownership is reachability, independent of strictness.
	g.received = map[string]bool{}
	for _, key := range outputs["protocol"] {
		g.markReceived(key, true)
	}
	protocol := g.received
	g.received = received
	sources := map[string][]byte{}
	tables, err := g.tableSources()
	if err != nil {
		return nil, err
	}
	if len(tables) > 0 {
		sources["tables"] = tables
	}
	names := make([]string, 0, len(outputs))
	for out := range outputs {
		names = append(names, out)
	}
	sort.Strings(names)
	for _, out := range names {
		g.ts.Reset()
		g.err = nil
		g.emitted = map[string]bool{}
		g.active = map[string]bool{}
		g.shared = map[string]bool{}
		g.tsImports = map[string]bool{}
		g.tsNames = map[string]string{}
		if out != "protocol" {
			g.shared = protocol
		}
		for _, key := range outputs[out] {
			g.emitTS(key)
		}
		if g.err != nil {
			return nil, g.err
		}
		var header strings.Builder
		header.WriteString("// Code generated by contractgen; DO NOT EDIT.\nimport { z } from 'zod'\n")
		imports := make([]string, 0, len(g.tsImports))
		for name := range g.tsImports {
			imports = append(imports, name)
		}
		sort.Strings(imports)
		if len(imports) > 0 {
			fmt.Fprintf(&header, "import { %s } from '@demicodes/protocol'\n", strings.Join(imports, ", "))
		}
		sources[out] = append([]byte(header.String()), g.ts.Bytes()...)
	}
	return sources, nil
}

// writeTS maps root output names to the frontend's established destinations.
func (g *generator) writeTS(p *packages.Package, sources map[string][]byte, verify bool) error {
	dir := filepath.Dir(p.GoFiles[0])
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			break
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return fmt.Errorf("module root not found")
		}
		dir = parent
	}
	outputs := make([]string, 0, len(sources))
	for out := range sources {
		outputs = append(outputs, out)
	}
	sort.Strings(outputs)
	for _, out := range outputs {
		filename := "contracts.ts"
		destination := out
		if out == "tables" {
			filename = "tables.ts"
			destination = "protocol"
		}
		if strings.HasPrefix(out, "plugin-") {
			filename = "plugin.ts"
		}
		dest := filepath.Join(dir, "packages", destination, "src", "generated", filename)
		if out == "web" {
			filename = "web-api.ts"
			dest = filepath.Join(dir, "packages", "web", "src", "api", "generated", filename)
		}
		if g.tsDir != "" {
			dest = filepath.Join(g.tsDir, destination, filename)
		}
		if !verify {
			if err := os.MkdirAll(filepath.Dir(dest), 0755); err != nil {
				return err
			}
		}
		if err := writeGenerated(dest, sources[out], verify); err != nil {
			return err
		}
	}
	return nil
}
