package main

import (
	"context"
	"fmt"
	"go/types"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"slices"
	"sort"
	"strings"

	"github.com/wspl/demi/tools/contractgen/pagemeta"
	"golang.org/x/tools/go/packages"
)

func isPointer(t types.Type) bool { _, ok := t.(*types.Pointer); return ok }

// check rejects contract declarations that cannot be represented faithfully.
func (g *generator) check(p *packages.Package) error {
	for _, name := range g.order {
		if err := g.checkDefinition(p, name); err != nil {
			return err
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

	for _, key := range []string{
		"object",
		"default",
		"nullable",
		"strict",
		"tolerant",
		"timestamp",
		"base64",
		"table",
		"schema",
		"schema-primitive",
		"flatten",
	} {
		if m[key] != "" {
			return fmt.Errorf("%s takes no arguments", key)
		}
	}
	if err := checkMarkArguments(m); err != nil {
		return err
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
func (g *generator) tsSources(pages ...pagemeta.Page) (map[string][]byte, error) {
	pageNames := map[string]map[string]bool{}
	for _, page := range pages {
		names, err := pageTypes(page, nil)
		if err != nil {
			return nil, err
		}
		pageNames[strings.TrimPrefix(page.Package, pageScope)] = names
	}
	g.tsExports = map[string]map[string]bool{}
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
		source, err := g.tsOutputSource(out, outputs[out], pageNames[out], received, protocol)
		if err != nil {
			return nil, err
		}
		sources[out] = source
	}
	return sources, nil
}

// writeTS maps root output names to the frontend's established destinations.
func (g *generator) writeTS(ctx context.Context, p *packages.Package, verify bool) error {
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
	var pages []pagemeta.Page
	if g.tsDir == "" {
		var err error
		pages, err = readPages(ctx, dir)
		if err != nil {
			return err
		}
	}
	sources, err := g.tsSources(pages...)
	if err != nil {
		return err
	}
	if g.tsDir == "" {
		if err := g.pageSources(pages, sources); err != nil {
			return err
		}
	}
	outputs := make([]string, 0, len(sources))
	for out := range sources {
		outputs = append(outputs, out)
	}
	sort.Strings(outputs)
	for _, out := range outputs {
		if err := g.writeTSOutput(dir, out, pages, sources[out], verify); err != nil {
			return err
		}
	}
	if g.tsDir == "" {
		return writeRegistries(dir, pages, verify)
	}
	return nil
}

func (g *generator) checkDefinition(p *packages.Package, name string) error {
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
	if err := g.checkUnion(d, name, fail); err != nil {
		return err
	}
	if err := g.checkVariant(d, fail); err != nil {
		return err
	}
	if root, ok := d.marks["root"]; ok {
		args := bounds(root)
		if args["direction"] != "receive" && args["direction"] != "send" &&
			(args["direction"] != "" || args["output"] != "") {
			return fail("root requires direction=receive|send")
		}
		if out := args["output"]; out != "" && out != "protocol" && out != "web" &&
			!regexp.MustCompile(`^plugin-[a-z][a-z0-9-]*$`).MatchString(out) {
			return fail("invalid root output")
		}
	}
	if has(d.marks, "codec") {
		if err := checkCodec(d, g.msgReach[d.key]); err != nil {
			return fail(err.Error())
		}
		return nil
	}
	if st, ok := g.object(d); ok {
		return g.checkObject(d, st, fail)
	}
	if !has(d.marks, "union") {
		if err := g.checkType(d.typ.Underlying()); err != nil {
			return fail(err.Error())
		}
	}
	return nil
}

func (g *generator) checkUnion(d *definition, name string, fail func(string) error) error {
	if !has(d.marks, "union") {
		return nil
	}

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

	return nil
}

func (g *generator) checkVariant(d *definition, fail func(string) error) error {
	variant := d.marks["variant"]
	if variant == "" {
		return nil
	}
	parts := strings.Fields(variant)
	if len(parts) < 1 || len(parts) > 2 || g.defs[parts[0]] == nil || !has(g.defs[parts[0]].marks, "union") {
		return fail("variant requires a known union and tag")
	}
	if _, object := d.typ.Underlying().(*types.Struct); !object {
		basic, scalar := d.typ.Underlying().(*types.Basic)
		if g.defs[parts[0]].marks["union"] != "untagged" || !scalar ||
			basic.Info()&(types.IsString|types.IsBoolean|types.IsInteger|types.IsFloat) == 0 {
			return fail("variant requires an object or an untagged scalar")
		}
	}
	if iface, ok := g.defs[parts[0]].typ.Underlying().(*types.Interface); ok && types.Implements(d.typ, iface) {
		return fail("variant sealing method must have a pointer receiver")
	}

	return nil
}

func (g *generator) checkObject(d *definition, st *types.Struct, fail func(string) error) error {
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
		if err := g.checkField(d, st, i, seen, fail); err != nil {
			return err
		}
	}
	return nil
}

func (g *generator) checkField(
	d *definition,
	st *types.Struct,
	i int,
	seen map[string]bool,
	fail func(string) error,
) error {
	f := st.Field(i)
	m := d.fields[f.Name()]
	if err := checkMarks(m); err != nil {
		return fail(f.Name() + ": " + err.Error())
	}
	if err := checkRuleType(f.Type(), m, true); err != nil {
		return fail(f.Name() + ": " + err.Error())
	}
	if err := g.checkFieldRules(f, m, fail); err != nil {
		return err
	}
	tag := reflect.StructTag(st.Tag(i)).Get("json")
	if child := g.optionalObject(f); child != nil {
		return g.checkOptionalField(d, f, child, tag, m, seen, fail)
	}
	if has(m, "flatten") {
		u := g.flattenedUnion(d, f)
		if g.adjacentUnion(d) != nil || u == nil || !has(u.marks, "union") ||
			bounds(u.marks["union"])["content"] == "" ||
			tag != "" ||
			!f.Exported() ||
			len(m) != 1 {
			return fail(
				f.Name() + ": flatten requires an adjacent union field without a JSON tag or other rules",
			)
		}
		for _, key := range []string{bounds(u.marks["union"])["tag"], bounds(u.marks["union"])["content"]} {
			if seen[key] {
				return fail(f.Name() + ": duplicate flattened field " + key)
			}
			seen[key] = true
		}
		return nil
	}
	if err := g.checkFieldTag(d, f, tag, m, seen, fail); err != nil {
		return err
	}
	if err := g.checkType(f.Type()); err != nil {
		return fail(f.Name() + ": " + err.Error())
	}
	return nil
}

func (g *generator) checkFieldRules(f *types.Var, m map[string]string, fail func(string) error) error {
	fieldType := f.Type()
	for {
		pointer, ok := fieldType.(*types.Pointer)
		if !ok {
			break
		}
		fieldType = pointer.Elem()
	}
	named, ok := fieldType.(*types.Named)
	if !ok {
		return nil
	}
	if child := g.defs[typeKey(named)]; child != nil {
		if has(child.marks, "integer") && (has(m, "nullable") || has(m, "timestamp")) ||
			has(m, "integer") && has(child.marks, "timestamp") {
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

	return nil
}

func (g *generator) checkFieldTag(
	d *definition,
	f *types.Var,
	tag string,
	m map[string]string,
	seen map[string]bool,
	fail func(string) error,
) error {
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
			return fail(
				f.Name() + ": optional values use pointers, default-false booleans, or omitempty collections",
			)
		}
	}
	if has(m, "nullable") && len(parts) > 1 && (parts[1] != "omitempty" || !isPointer(f.Type())) {
		return fail(f.Name() + ": optional nullable fields require a pointer with omitempty")
	}

	return nil
}

func checkMarkArguments(m map[string]string) error {
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
	return checkBoundArguments(m)
}

func (g *generator) tsOutputSource(
	out string,
	roots []string,
	pageNames, received, protocol map[string]bool,
) ([]byte, error) {
	g.pageNames = pageNames
	g.tsOutput = out
	g.received = received
	if strings.HasPrefix(out, "plugin-") {
		// A plugin page's tolerance comes from that page's own uses, not
		// from unrelated REST or other page roots that reach the type.
		g.received = map[string]bool{}
		for _, key := range roots {
			if bounds(g.defs[key].marks["root"])["direction"] == "receive" {
				g.markReceived(key, true)
			}
		}
	}
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
	for _, key := range roots {
		g.emitTS(key)
	}
	if g.err != nil {
		return nil, g.err
	}
	var header strings.Builder
	header.WriteString(tsHeader)
	header.WriteString("import { z } from 'zod'\n")
	imports := make([]string, 0, len(g.tsImports))
	for name := range g.tsImports {
		imports = append(imports, name)
	}
	sort.Strings(imports)
	if len(imports) > 0 {
		fmt.Fprintf(&header, "import { %s } from '@demicodes/protocol'\n", strings.Join(imports, ", "))
	}
	source := append([]byte(header.String()), g.ts.Bytes()...)
	g.tsExports[out] = map[string]bool{}
	for key := range g.emitted {
		d := g.defs[key]
		if g.exportsTS(d) {
			g.tsExports[out][tsName(d.name)] = g.received[key]
		}
	}
	return source, nil
}

func (g *generator) writeTSOutput(dir, out string, pages []pagemeta.Page, source []byte, verify bool) error {
	filename := "contracts.ts"
	destination := out
	if out == "tables" {
		filename = "tables.ts"
		destination = "protocol"
	}
	if strings.HasPrefix(out, "plugin-") ||
		slices.ContainsFunc(pages, func(page pagemeta.Page) bool { return page.Package == pageScope+out }) {
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
		if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
			return err
		}
	}
	if err := writeGenerated(dest, source, verify); err != nil {
		return err
	}
	return nil
}

func (g *generator) checkOptionalField(
	d *definition,
	f *types.Var,
	child *definition,
	tag string,
	m map[string]string,
	seen map[string]bool,
	fail func(string) error,
) error {
	if !f.Exported() || tag != "" || len(m) != 0 || g.adjacentUnion(d) != nil || g.tupleUnion(d) {
		return fail(
			f.Name() + ": optional embedded objects cannot have tags, rules, or tuple/adjacent content",
		)
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
	return nil
}

func checkBoundArguments(m map[string]string) error {
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
			if !ok || (name != "min" && name != "max") ||
				!regexp.MustCompile(`^-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$`).MatchString(value) {
				return fmt.Errorf("invalid %s argument %q", key, arg)
			}
		}
	}
	return nil
}
