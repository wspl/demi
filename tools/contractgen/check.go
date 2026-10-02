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
			iface, ok := d.typ.Underlying().(*types.Interface)
			if !ok || iface.NumMethods() != 1 || iface.Method(0).Exported() {
				return fail("union must be an interface with one unexported sealing method")
			}
			if d.marks["msgpack"] == "tuple" && (d.marks["union"] == "untagged" || g.unionKind(d) == "bool") {
				return fail("tuple unions require string tags")
			}
			if bounds(d.marks["union"])["tag"] == "" && d.marks["union"] != "untagged" {
				return fail("union requires tag=<field>")
			}
			signature := iface.Method(0).Type().(*types.Signature)
			if signature.Params().Len() != 0 || signature.Results().Len() != 0 {
				return fail("union sealing method must have no parameters or results")
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
		if st, ok := g.object(d); ok {
			if g.err != nil {
				return g.err
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
				tag := reflect.StructTag(st.Tag(i)).Get("json")
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
				if len(parts) > 1 && g.tupleUnion(d) {
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
				if has(m, "nullable") && len(parts) > 1 {
					return fail(f.Name() + ": nullable must be required")
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
	for _, key := range []string{"nullable", "strict", "tolerant", "timestamp", "base64", "table", "schema"} {
		if m[key] != "" {
			return fmt.Errorf("%s takes no arguments", key)
		}
	}
	for _, key := range []string{"union", "root"} {
		allowed := map[string]bool{"tag": true}
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
		if !types.Identical(t.Key(), types.Typ[types.String]) {
			return fmt.Errorf("record keys must be strings")
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
				g.markReceived(key)
			}
		}
	}
	received := g.received
	// Protocol ownership is reachability, independent of strictness.
	g.received = map[string]bool{}
	for _, key := range outputs["protocol"] {
		g.markReceived(key)
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

// checkMsgpackShape refuses JSON's opaque token stream on a different codec.
func checkMsgpackShape(t types.Type) error {
	if isJSON(t) {
		return fmt.Errorf("opaque JSON has no MessagePack representation")
	}
	switch t := t.(type) {
	case *types.Pointer:
		return checkMsgpackShape(t.Elem())
	case *types.Slice:
		return checkMsgpackShape(t.Elem())
	case *types.Map:
		return checkMsgpackShape(t.Elem())
	case *types.Struct:
		for i := 0; i < t.NumFields(); i++ {
			if err := checkMsgpackShape(t.Field(i).Type()); err != nil {
				return fmt.Errorf("%s: %w", t.Field(i).Name(), err)
			}
		}
	}
	return nil
}
