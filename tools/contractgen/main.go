// Command contractgen derives boundary codecs, validation and Zod from Go types.
package main

import (
	"bytes"
	"context"
	"flag"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"go/types"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"

	"golang.org/x/tools/go/packages"
)

type definition struct {
	unions            []string
	description       string
	fieldDescriptions map[string]string
	base              string
	position          string
	name              string
	key               string
	typ               *types.Named
	marks             map[string]string
	fields            map[string]map[string]string
}

type generator struct {
	pageNames map[string]bool
	tsOutput  string
	tsExports map[string]map[string]bool
	jsonReach map[string]bool
	msgReach  map[string]bool
	tables    []*table
	defs      map[string]*definition
	order     []string
	goCode    bytes.Buffer
	ts        bytes.Buffer
	emitted   map[string]bool
	active    map[string]bool
	received  map[string]bool
	err       error
	tsDir     string
	current   *types.Package
	imports   map[string]string
	shared    map[string]bool
	tsImports map[string]bool
	tsNames   map[string]string
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	ts := flag.Bool("ts", false, "emit TypeScript instead of Go")
	verify := flag.Bool("check", false, "verify generated files without writing")
	tsDir := flag.String("ts-dir", "", "override TypeScript destination directory (fixture generation)")
	flag.Parse()
	patterns := flag.Args()
	if len(patterns) == 0 {
		patterns = []string{"."}
		if *ts {
			patterns = []string{"./..."}
		}
	}
	return generate(context.Background(), patterns, *ts, *tsDir, *verify)
}

// generateBatch strips ordinary function bodies so generation also works before
// generated methods exist. Generic declarations retain the bodies Go requires.
// Type errors in declarations remain fatal.
func generateBatch(
	ctx context.Context,
	patterns []string,
	ts bool,
	tsDir string,
	verify bool,
	stripped map[string]bool,
	overlay map[string][]byte,
	emit func(string, []byte) error,
) error {
	config := &packages.Config{
		Context: ctx,
		Overlay: overlay,
		Mode: packages.NeedName | packages.NeedFiles | packages.NeedSyntax | packages.NeedTypes | packages.NeedTypesInfo |
			packages.NeedImports | packages.NeedDeps,
		ParseFile: func(fset *token.FileSet, filename string, src []byte) (*ast.File, error) {
			return parseContractFile(fset, filename, src, stripped)
		},
	}
	pkgs, err := packages.Load(config, patterns...)
	if err != nil {
		return err
	}
	g := generator{defs: map[string]*definition{}, tsDir: tsDir}
	collected, err := g.collectPackages(pkgs)
	if err != nil {
		return err
	}
	if len(g.order) == 0 && len(g.tables) == 0 {
		return nil
	}
	if err := g.prepareContracts(); err != nil {
		return err
	}
	if err := g.check(collected[0]); err != nil {
		return err
	}
	jsonSchemas, err := g.jsonSchemas(false)
	if err != nil {
		return err
	}
	pluginSchemas, err := g.jsonSchemas(true)
	if err != nil {
		return err
	}
	if ts {
		return g.writeTS(ctx, collected[0], verify)
	}
	if _, err := g.tsSources(); err != nil {
		return err
	}
	return g.emitPackages(pkgs, jsonSchemas, pluginSchemas, emit)
}

// markers reads the contract annotation attached to a Go declaration.
func markers(doc *ast.CommentGroup) map[string]string {
	out := map[string]string{}
	if doc == nil {
		return out
	}
	for _, line := range doc.List {
		text := strings.TrimSpace(strings.TrimPrefix(line.Text, "//"))
		if !strings.HasPrefix(text, "+demi:") {
			continue
		}
		key, value, _ := strings.Cut(strings.TrimPrefix(text, "+demi:"), " ")
		switch key {
		case "integer",
			"default",
			"codec",
			"flatten",
			"union",
			"variant",
			"nullable",
			"length",
			"range",
			"enum",
			"pattern",
			"timestamp",
			"check",
			"id",
			"base64",
			"msgpack",
			"strict",
			"tolerant",
			"root",
			"format",
			"table",
			"schema",
			"schema-primitive":
		default:
			out["!error"] = "unsupported marker: " + key
		}
		if _, duplicate := out[key]; duplicate {
			out["!error"] = "duplicate marker: " + key
		}
		out[key] = value
	}
	return out
}

func has(m map[string]string, key string) bool {
	_, ok := m[key]
	return ok
}
func quote(s string) string                       { return strconv.Quote(s) }
func typeKey(t *types.Named) string               { return t.Obj().Pkg().Path() + "." + t.Obj().Name() }
func (g *generator) typeName(t types.Type) string { return types.TypeString(t, g.qualifier) }
func (g *generator) qualifier(p *types.Package) string {
	if p != nil && p.Path() == "encoding/json" {
		g.imports[p.Path()] = "json"
		return "json"
	}
	if p == nil || p == g.current {
		return ""
	}
	if alias := g.imports[p.Path()]; alias != "" {
		return alias
	}
	alias := p.Name()
	used := map[string]bool{"fmt": true, "json": true, "contract": true, "sort": true, "reflect": true}
	for _, a := range g.imports {
		used[a] = true
	}
	for used[alias] {
		alias += "pkg"
	}
	g.imports[p.Path()] = alias
	return alias
}

func (g *generator) prefix(t *types.Named) string {
	alias := g.qualifier(t.Obj().Pkg())
	if alias != "" {
		return alias + "."
	}
	return ""
}
func (g *generator) line(s string, args ...any) { fmt.Fprintf(&g.goCode, s+"\n", args...) }
func schema(name string) string {
	name = tsName(name)
	return strings.ToLower(name[:1]) + name[1:] + "Schema"
}

func bounds(s string) map[string]string {
	out := map[string]string{}
	for _, field := range strings.Fields(s) {
		if field == "chars" {
			continue
		}
		k, v, ok := strings.Cut(field, "=")
		if !ok {
			continue
		}
		out[k] = v
	}
	return out
}

func (g *generator) decoder(t types.Type) string {
	if isJSON(t) {
		return "contract.JSON"
	}
	switch t := t.(type) {
	case *types.Pointer:
		return "func(b []byte)(" + g.typeName(t) + ",error){ return contract.Pointer(b," + g.decoder(t.Elem()) + ") }"
	case *types.Map:
		if !types.Identical(t.Key(), types.Typ[types.String]) {
			return "func(b []byte)(" + g.typeName(
				t,
			) + ",error){return contract.KeyedRecord[" + g.typeName(
				t.Key(),
			) + "](b," + g.decoder(
				t.Elem(),
			) + "," + strconv.FormatBool(
				isPointer(t.Elem()),
			) + ")}"
		}
		return "func(b []byte)(" + g.typeName(
			t,
		) + ",error){ return contract.Record(b," + g.decoder(
			t.Elem(),
		) + "," + strconv.FormatBool(
			isPointer(t.Elem()),
		) + ") }"
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return "contract.Bytes"
		}
		return "func(b []byte)(" + g.typeName(t) + ",error){ return contract.List(b," + g.decoder(t.Elem()) + ") }"
	case *types.Named:
		if _, ok := t.Underlying().(*types.Interface); ok {
			return g.prefix(t) + goName("Decode", t.Obj().Name())
		}
	}
	return "contract.Decode[" + g.typeName(t) + "]"
}

func (g *generator) emitGo(d *definition) {
	if has(d.marks, "codec") {
		return
	}
	name := d.name
	if has(d.marks, "union") {
		g.emitGoUnion(d)
		return
	}
	if has(d.marks, "id") {
		g.line("func %s(value string)(%s,error){", goName("Parse", name), name)
		g.normalizeText(d, "value", "return \"\",err")
		g.line("v:=%s(value);if err:=v.Validate();err!=nil{return \"\",err};return v,nil}", name)
	}
	st, isStruct := g.object(d)
	tag, variant, kind := g.variantWire(d)
	if !g.emitUnionSeals(d) {
		return
	}

	if g.jsonReach[d.key] {
		g.line("func %s(data []byte)(%s,error){return contract.Decode[%s](data)}", goName("Decode", name), name, name)
	}

	g.line("func(v %s)Validate()error{return %s(v,0)}", name, goName("contractValidate", name))
	g.line(
		"func %s(v %s,depth int)error{if depth>1000{return fmt.Errorf(\"validation nesting exceeds 1000\")}",
		goName("contractValidate", name),
		name,
	)
	if isStruct {
		g.emitObjectValidation(d)
	} else {
		g.validation(d.typ.Underlying(), "v", quote(""), d.marks)
	}
	if custom := d.marks["check"]; custom != "" {
		g.line("if err:=%s(v);err!=nil{return err}", custom)
	}
	g.line("return nil }")
	if !g.jsonReach[d.key] {
		return
	}
	g.emitJSONCodec(d, st, isStruct, tag, variant, kind)
}

func (g *generator) validation(t types.Type, expr, path string, m map[string]string) {
	if isJSON(t) {
		g.line("if err:=contract.CheckJSON(%s);err!=nil{return contract.At(%s,err)}", expr, path)
		return
	}
	if ptr, ok := t.(*types.Pointer); ok {
		if !has(m, "nullable") && !has(m, "optional") {
			g.line("if %s==nil{return contract.At(%s,fmt.Errorf(\"required pointer is nil\"))}", expr, path)
		}
		g.line("if %s!=nil{", expr)
		elementMarks := maps.Clone(m)
		delete(elementMarks, "optional")
		g.validation(ptr.Elem(), "(*"+expr+")", path, elementMarks)
		g.line("}")
		return
	}
	if named, ok := t.(*types.Named); ok {
		if g.validateNamed(named, t, expr, path, m) {
			return
		}
	}
	if record, ok := t.(*types.Map); ok {
		if !has(m, "optional") {
			g.line("if %s==nil{return contract.At(%s,fmt.Errorf(\"required record is nil\"))}", expr, path)
		}
		if types.Identical(record.Key(), types.Typ[types.String]) {
			g.line(
				"for key,item:=range %s{_=item;if err:=contract.Text(key,0,-1,\"\");err!=nil{return contract.At(%s,err)}",
				expr,
				path,
			)
		} else {
			g.line("for key,item:=range %s{_=item", expr)
			g.validation(record.Key(), "key", "fmt.Sprintf(\"%s[%q]\","+path+",key)", map[string]string{})
		}
		g.validation(record.Elem(), "item", "fmt.Sprintf(\"%s[%q]\","+path+",key)", map[string]string{"nullable": ""})
		g.line("}")
	}
	if slice, ok := t.(*types.Slice); ok {
		if !has(m, "optional") {
			g.line("if %s==nil{return contract.At(%s,fmt.Errorf(\"required array is nil\"))}", expr, path)
		}
		g.line("for i,item:=range %s { _=i;_=item", expr)
		g.validation(slice.Elem(), "item", "fmt.Sprintf(\"%s[%d]\","+path+",i)", map[string]string{})
		g.line("}")
	}
	g.rules(t, expr, path, m)
}

func (g *generator) rules(t types.Type, expr, path string, m map[string]string) {
	if b, ok := t.Underlying().(*types.Basic); ok && b.Info()&types.IsFloat != 0 {
		g.imports["math"] = "math"
		g.line(
			"if math.IsNaN(float64(%s)) || math.IsInf(float64(%s),0){return "+
				"contract.At(%s,fmt.Errorf(\"number must be finite\"))}",
			expr,
			expr,
			path,
		)
	}
	g.formatRules(expr, path, m)
	if has(m, "base64") && !emptyCollection(t) {
		g.line("if err:=contract.Base64(string(%s));err!=nil{return contract.At(%s,err)}", expr, path)
	}
	if has(m, "timestamp") && !integerTimestamp(t) {
		g.line("if err:=contract.Timestamp(string(%s));err!=nil{return contract.At(%s,err)}", expr, path)
	}
	if values := m["enum"]; values != "" {
		var vals []string
		for _, v := range strings.Fields(values) {
			vals = append(vals, quote(v))
		}
		g.line(
			"switch string(%s){case %s:default:return contract.At(%s,fmt.Errorf(\"unknown value\"))}",
			expr,
			strings.Join(vals, ","),
			path,
		)
	}
	g.lengthRules(t, expr, path, m)
	if slices.Contains(strings.Fields(m["range"]), "schema-only") {
		return
	}
	for _, key := range []string{"min", "max"} {
		if bound := bounds(m["range"])[key]; bound != "" {
			op := "<"
			if key == "max" {
				op = ">"
			}
			g.line("if %s %s %s{return contract.At(%s,fmt.Errorf(\"outside numeric bounds\"))}", expr, op, bound, path)
		}
	}
}

// writeGenerated supports reproducibility checks without mutating the checkout.
func writeGenerated(path string, code []byte, verify bool) error {
	if verify {
		existing, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if !bytes.Equal(existing, code) {
			return fmt.Errorf("%s: generated file is stale", path)
		}
		return nil
	}
	return os.WriteFile(path, code, 0o644)
}

func isJSON(t types.Type) bool {
	if a, ok := t.(*types.Alias); ok {
		return a.Obj().Pkg() != nil && a.Obj().Pkg().Path() == "encoding/json" && a.Obj().Name() == "RawMessage"
	}
	n, ok := t.(*types.Named)
	return ok && n.Obj().Pkg() != nil && n.Obj().Pkg().Path() == "encoding/json" && n.Obj().Name() == "RawMessage"
}

// inheritFields retains field annotations through concrete named derivations.
func (g *generator) inheritFields(d *definition) {
	if base := g.defs[d.base]; base != nil {
		g.inheritFields(base)
		for name, marks := range base.fields {
			d.fields[name] = marks
			d.fieldDescriptions[name] = base.fieldDescriptions[name]
		}
	}
}

// contractDescription keeps contract documentation as schema product text,
// excluding generator and Go tooling directives rather than rewriting prose.
func contractDescription(doc *ast.CommentGroup) string {
	if doc == nil {
		return ""
	}
	var lines []string
	for _, comment := range doc.List {
		text := strings.TrimPrefix(comment.Text, "//")
		text = strings.TrimPrefix(text, " ")
		directive := strings.TrimSpace(text)
		if strings.HasPrefix(directive, "+demi:") || strings.HasPrefix(directive, "sumtype:") ||
			strings.HasPrefix(directive, "go:") {
			continue
		}
		if strings.HasPrefix(text, "/*") {
			text = strings.TrimSuffix(strings.TrimPrefix(text, "/*"), "*/")
		}
		lines = append(lines, text)
	}
	// A description is the comment without its delimiter and one leading space per
	// line, trimmed of outer ASCII whitespace. CommentGroup.Text also collapses
	// blank lines and indentation, which would change the help/model text.
	return strings.Trim(strings.Join(lines, "\n"), " \t\n\r\v\f")
}

// parseContractFile retains generic bodies because Go requires them when checking declarations.
func parseContractFile(fset *token.FileSet, filename string, src []byte, stripped map[string]bool) (*ast.File, error) {
	file, err := parser.ParseFile(fset, filename, src, parser.ParseComments)
	if err != nil {
		return nil, err
	}
	if stripped[filename] {
		file.Decls = nil
		file.Imports = nil
		return file, nil
	}
	if filepath.Base(filename) == "contract_gen.go" {
		return file, nil
	}
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if !ok || fn.Type.TypeParams.NumFields() > 0 {
			continue
		}
		if fn.Recv.NumFields() > 0 {
			receiver := ast.Unparen(fn.Recv.List[0].Type)
			if pointer, ok := receiver.(*ast.StarExpr); ok {
				receiver = ast.Unparen(pointer.X)
			}
			switch receiver.(type) {
			case *ast.IndexExpr, *ast.IndexListExpr:
				continue
			}
		}
		fn.Body = nil
	}
	return file, nil
}

// collectPackage reports whether the package contributed marked declarations; loadErr retains alias diagnostics.
func (g *generator) collectPackage(p *packages.Package, unusedImport *regexp.Regexp, loadErr *error) bool {
	var defs []*definition
	marked := false
	for _, file := range p.Syntax {
		for _, decl := range file.Decls {
			general, ok := decl.(*ast.GenDecl)
			if !ok {
				continue
			}
			if !g.collectDeclaration(p, general, &defs, &marked, loadErr) {
				return false
			}
		}
	}
	if !marked {
		return false
	}
	for _, problem := range p.Errors {
		if problem.Kind != packages.TypeError || !unusedImport.MatchString(problem.Msg) {
			*loadErr = fmt.Errorf("%s", problem)
			return false
		}
	}
	for _, d := range defs {
		g.defs[d.key] = d
		g.order = append(g.order, d.key)
	}
	return true
}

// collectDeclaration stops the package visit on an invalid table and otherwise retains declaration order.
func (g *generator) collectDeclaration(
	p *packages.Package,
	general *ast.GenDecl,
	defs *[]*definition,
	marked *bool,
	loadErr *error,
) bool {
	for _, spec := range general.Specs {
		if value, ok := spec.(*ast.ValueSpec); ok {
			doc := general.Doc
			if value.Doc != nil {
				doc = value.Doc
			}
			marks := markers(doc)
			if len(marks) > 0 {
				*marked = true
				table, err := readTable(p, value, marks)
				if err != nil {
					*loadErr = err
					return false
				}
				g.tables = append(g.tables, table)
			}
			continue
		}

		spec, ok := spec.(*ast.TypeSpec)
		if !ok {
			continue
		}
		doc := general.Doc
		if spec.Doc != nil {
			doc = spec.Doc
		}
		marks := markers(doc)
		if len(marks) > 0 {
			*marked = true
		}
		obj := p.TypesInfo.Defs[spec.Name]
		if obj == nil {
			continue
		}
		named, ok := obj.Type().(*types.Named)
		if !ok {
			if has(marks, "root") {
				*loadErr = fmt.Errorf(
					"%s: %s: aliases unsupported",
					p.Fset.Position(spec.Pos()),
					spec.Name.Name,
				)
			}
			continue
		}
		d := readDefinition(p, spec, doc, named, marks, marked)
		*defs = append(*defs, d)
	}
	return true
}

func readDefinition(
	p *packages.Package,
	spec *ast.TypeSpec,
	doc *ast.CommentGroup,
	named *types.Named,
	marks map[string]string,
	marked *bool,
) *definition {
	d := &definition{
		description:       contractDescription(doc),
		fieldDescriptions: map[string]string{},
		position:          p.Fset.Position(spec.Pos()).String(),
		name:              spec.Name.Name,
		key:               typeKey(named),
		typ:               named,
		marks:             marks,
		fields:            map[string]map[string]string{},
	}
	if source, ok := p.TypesInfo.TypeOf(spec.Type).(*types.Named); ok {
		d.base = typeKey(source.Origin())
	}
	if st, ok := spec.Type.(*ast.StructType); ok {
		for _, field := range st.Fields.List {
			if len(field.Names) == 0 && len(markers(field.Doc)) > 0 {
				d.marks["!error"] = "markers on embedded fields are unsupported"
			}
			for _, name := range field.Names {
				d.fields[name.Name] = markers(field.Doc)
				d.fieldDescriptions[name.Name] = contractDescription(field.Doc)
				if len(d.fields[name.Name]) > 0 {
					*marked = true
				}
			}
		}
	}
	if st, ok := named.Underlying().(*types.Struct); ok {
		for i := 0; i < st.NumFields(); i++ {
			if d.fields[st.Field(i).Name()] == nil {
				d.fields[st.Field(i).Name()] = map[string]string{}
			}
		}
	}
	return d
}

func (g *generator) emitPackages(
	pkgs []*packages.Package,
	jsonSchemas, pluginSchemas map[string][]byte,
	emit func(string, []byte) error,
) error {
	for _, p := range pkgs {
		g.current = p.Types
		g.imports = map[string]string{}
		g.goCode.Reset()
		for _, name := range g.order {
			d := g.defs[name]
			if d.typ.Obj().Pkg() == p.Types {
				g.emitGo(d)
				if value, ok := jsonSchemas[d.key]; ok {
					g.imports["encoding/json"] = "json"
					g.line(
						"func %sJSONSchema() json.RawMessage { return json.RawMessage(%s) }",
						d.name,
						quote(string(value)),
					)
					g.line(
						"func %sPluginJSONSchema() json.RawMessage { return json.RawMessage(%s) }",
						d.name,
						quote(string(pluginSchemas[d.key])),
					)
				}
			}
		}
		for _, name := range g.order {
			d := g.defs[name]
			if d.typ.Obj().Pkg() == p.Types && g.msgReach[d.key] {
				g.emitMsgpack(d)
			}
		}
		if g.goCode.Len() == 0 {
			continue
		}
		code, err := g.goSource(p)
		if err != nil {
			return err
		}
		if err := emit(filepath.Join(filepath.Dir(p.GoFiles[0]), "contract_gen.go"), code); err != nil {
			return err
		}
	}
	return nil
}

func (g *generator) goSource(p *packages.Package) ([]byte, error) {
	var header strings.Builder
	fmt.Fprintf(&header, "// Code generated by contractgen; DO NOT EDIT.\n\npackage %s\nimport (\"fmt\";", p.Name)
	if g.imports["encoding/json"] != "" {
		header.WriteString("\"encoding/json\";")
	}
	header.WriteString("\"github.com/wspl/demi/internal/contract\";")
	paths := make([]string, 0, len(g.imports))
	for path := range g.imports {
		if path == "encoding/json" {
			continue
		}
		paths = append(paths, path)
	}
	sort.Strings(paths)
	for _, path := range paths {
		fmt.Fprintf(&header, "%s %s;", g.imports[path], quote(path))
	}
	header.WriteString(")\n")
	code, err := format.Source(append([]byte(header.String()), g.goCode.Bytes()...))
	if err != nil {
		return nil, fmt.Errorf("format generated code: %w", err)
	}
	return code, nil
}

func (g *generator) emitGoUnion(d *definition) {
	name := d.name
	if g.jsonReach[d.key] {
		g.emitJSONUnionDecode(d)
		// An interface cannot own UnmarshalJSON; the transport holder does.
		g.line(
			"type %sJSON struct { Value %s }; func(v *%sJSON) UnmarshalJSON(data "+
				"[]byte) error { value,err:=%s(data); if err==nil {v.Value=value}; "+
				"return err }; func(v %sJSON) MarshalJSON()([]byte,error){ if "+
				"err:=%s(v.Value);err!=nil{return nil,err}; return "+
				"contract.EncodeJSON(v.Value) }",
			name,
			name,
			name,
			goName("Decode", name),
			name,
			goName("Validate", name),
		)
	}
	g.line(
		"func %s(value %s)error{return %s(value,0)}",
		goName("Validate", name),
		name,
		goName("contractValidate", name),
	)
	g.line(
		"func %s(value %s,depth int)error{if depth>1000{return "+
			"fmt.Errorf(\"validation nesting exceeds 1000\")};switch v:=value.(type){",
		goName("contractValidate", name),
		name,
	)
	for _, v := range g.variants(d.key) {
		g.line(
			"case *%s: if v==nil{return fmt.Errorf(\"nil variant\")}; return %s(*v,depth+1)",
			v.name,
			goName("contractValidate", v.name),
		)
	}
	g.line("default:return fmt.Errorf(\"nil or unsupported %s\")}}", name)
}

func (g *generator) emitObjectValidation(d *definition) {
	validationStruct := d.typ.Underlying().(*types.Struct)
	for i := 0; i < validationStruct.NumFields(); i++ {
		f := validationStruct.Field(i)
		key := strings.Split(reflect.StructTag(validationStruct.Tag(i)).Get("json"), ",")[0]
		if u := g.adjacentUnion(d); u != nil {
			key = bounds(u.marks["union"])["content"]
		}
		m := maps.Clone(d.fields[f.Name()])
		if g.optionalObject(f) != nil || has(m, "default") {
			m["optional"] = ""
		}
		if len(strings.Split(reflect.StructTag(validationStruct.Tag(i)).Get("json"), ",")) > 1 {
			m["optional"] = ""
		}
		g.validation(f.Type(), "v."+f.Name(), quote(key), m)
	}
}

func (g *generator) emitJSONObjectDecode(d *definition, st *types.Struct, tag, variant, kind string) {
	name := d.name
	object := "obj"
	if st.NumFields() == 0 && tag == "" && has(d.marks, "tolerant") {
		object = "_"
	}
	g.line(
		"%s,err:=contract.Decode[map[string]json.RawMessage](data); if err!=nil{return err}; var next %s",
		object,
		name,
	)
	if !has(d.marks, "tolerant") {
		g.line("for key:=range obj{switch key{")
		keys := []string{}
		for _, key := range g.propertyNames(d, st) {
			keys = append(keys, quote(key))
		}
		if tag != "" {
			keys = append(keys, quote(tag))
		}
		if len(keys) > 0 {
			g.line("case %s:", strings.Join(keys, ","))
		}
		g.line("default:return contract.At(key,fmt.Errorf(\"unknown field\"))}}")
	}
	if tag != "" {
		g.line(
			"if raw,ok:=obj[%s];!ok { return fmt.Errorf(\"missing union tag\") } "+
				"else { value,err:=contract.Decode[%s](raw);if "+
				"err!=nil||value!=%s{return fmt.Errorf(\"invalid union tag\")} }",
			quote(tag),
			kind,
			tagLiteral(variant),
		)
	}
	for i := 0; i < st.NumFields(); i++ {
		g.emitJSONFieldDecode(d, st, i)
	}
}

func (g *generator) emitJSONFieldDecode(d *definition, st *types.Struct, i int) {
	f := st.Field(i)
	if child := g.optionalObject(f); child != nil {
		g.emitOptionalObjectDecode(f, child, false)
		return
	}
	if g.flattenedUnion(d, f) != nil {
		g.emitFlattenDecode(d, f, false)
		return
	}
	opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
	key := opts[0]
	optional := len(opts) > 1 || has(d.fields[f.Name()], "default")
	nullable := has(d.fields[f.Name()], "nullable")
	if optional && emptyCollection(f.Type()) {
		g.line("next.%s=make(%s,0)", f.Name(), g.typeName(f.Type()))
	}
	g.line("{raw,ok:=obj[%s];", quote(key))
	if !optional {
		g.line("if !ok{return contract.At(%s,fmt.Errorf(\"required field is absent\"))}", quote(key))
	}
	g.line("if ok {")
	if nullable {
		if pointer, ok := f.Type().(*types.Pointer); optional && ok && isPointer(pointer.Elem()) {
			g.line("if contract.IsNull(raw){next.%s=new(%s)}else{", f.Name(), g.typeName(pointer.Elem()))
		} else {
			g.line("if !contract.IsNull(raw) {")
		}
	}
	g.line(
		"value,err:=%s(raw); if err!=nil{return contract.At(%s,err)}; next.%s=value",
		g.integerDecoder(f.Type(), d.fields[f.Name()], false),
		quote(key),
		f.Name(),
	)
	if nullable {
		g.line("}")
	}
	g.line("}}")
}

// validateNamed reports whether named validation has completed the caller’s work for this value.
func (g *generator) validateNamed(named *types.Named, t types.Type, expr, path string, m map[string]string) bool {
	if d := g.defs[typeKey(named)]; d != nil && has(d.marks, "codec") {
		return true
	}
	if has(m, "optional") && emptyCollection(t) {
		g.line("{collection:=%s;if collection==nil{collection=make(%s,0)}", expr, g.typeName(t))
		required := maps.Clone(m)
		delete(required, "optional")
		g.validation(t, "collection", path, required)
		g.line("}")
		return true
	}
	if _, ok := named.Underlying().(*types.Interface); ok {
		if has(m, "nullable") {
			g.line("if %s!=nil{", expr)
		}
		if named.Obj().Pkg() == g.current {
			g.line(
				"if err:=%s(%s,depth+1);err!=nil{return contract.At(%s,err)}",
				goName("contractValidate", named.Obj().Name()),
				expr,
				path,
			)
		} else {
			g.line(
				"if err:=%s%s(%s);err!=nil{return contract.At(%s,err)}",
				g.prefix(named),
				goName("Validate", named.Obj().Name()),
				expr,
				path,
			)
		}
		if has(m, "nullable") {
			g.line("}")
		}
		return true
	}
	if named.Obj().Pkg() == g.current {
		g.line(
			"if err:=%s(%s,depth+1);err!=nil{return contract.At(%s,err)}",
			goName("contractValidate", named.Obj().Name()),
			expr,
			path,
		)
	} else {
		g.line("if err:=%s.Validate();err!=nil{return contract.At(%s,err)}", expr, path)
	}
	return false
}

func (g *generator) lengthRules(t types.Type, expr, path string, m map[string]string) {
	b := bounds(m["length"])
	_, isString := t.Underlying().(*types.Basic)
	if isString && t.Underlying().(*types.Basic).Info()&types.IsString != 0 {
		minimum, maximum := b["min"], b["max"]
		if minimum == "" {
			minimum = "0"
		}
		if maximum == "" {
			maximum = "-1"
		}
		g.line(
			"if err:=contract.Text(string(%s),%s,%s,%s);err!=nil{return contract.At(%s,err)}",
			expr,
			minimum,
			maximum,
			quote(m["pattern"]),
			path,
		)
		return
	}
	if minimum := b["min"]; minimum != "" {
		g.line("if len(%s)<%s{return contract.At(%s,fmt.Errorf(\"too few items\"))}", expr, minimum, path)
	}
	if maximum := b["max"]; maximum != "" {
		g.line("if len(%s)>%s{return contract.At(%s,fmt.Errorf(\"too many items\"))}", expr, maximum, path)
	}
}

func (g *generator) prepareContracts() error {
	sort.Strings(g.order)
	for _, key := range g.order {
		g.inheritFields(g.defs[key])
	}
	concrete := g.order[:0]
	for _, key := range g.order {
		d := g.defs[key]
		if d.typ.TypeParams().Len() > 0 && len(d.marks) == 0 {
			continue
		}
		concrete = append(concrete, key)
	}
	g.order = concrete
	g.normalizeVariants()
	if err := g.retainContracts(); err != nil {
		return err
	}

	return nil
}

// emitUnionSeals reports failure without emitting the remaining methods after an invalid sealing signature.
func (g *generator) emitUnionSeals(d *definition) bool {
	name := d.name
	for _, union := range d.unions {
		seal, err := unionSeal(g.defs[union].typ)
		if err != nil {
			g.err = fmt.Errorf("%s: %s: %w", d.position, d.name, err)
			return false
		}
		method := seal.Name()
		if obj, _, _ := types.LookupFieldOrMethod(
			types.NewPointer(d.typ),
			true,
			d.typ.Obj().Pkg(),
			method,
		); obj == nil {
			g.line("func (*%s) %s() {}", name, method)
		}
	}

	return true
}

func (g *generator) formatRules(expr, path string, m map[string]string) {
	if format := m["format"]; format != "" {
		if format == "trimmed" {
			g.line(
				"if contract.Trim(string(%s))!=string(%s){return contract.At(%s,fmt.Errorf(\"text is not trimmed\"))}",
				expr,
				expr,
				path,
			)
		} else {
			helper := "Email"
			if format == "http-url" {
				helper = "HTTPURL"
			}
			g.line(
				"if value,err:=contract.%s(string(%s));err!=nil{return contract.At(%s,"+
					"err)}else if value!=string(%s){return contract.At(%s,fmt.Errorf(\"text "+
					"is not canonical\"))}",
				helper,
				expr,
				path,
				expr,
				path,
			)
		}
	}
}

func (g *generator) emitJSONUnionDecode(d *definition) {
	name := d.name
	tag := bounds(d.marks["union"])["tag"]
	g.line("func %s(data []byte)(%s,error) {", goName("Decode", name), name)
	if d.marks["union"] == "untagged" {
		g.line("if err:=contract.CheckJSON(data);err!=nil{return nil,err}")
		for _, v := range g.variants(d.key) {
			g.line("if value,err:=contract.Decode[%s](data);err==nil{return &value,nil}", v.name)
		}
		g.line("return nil,fmt.Errorf(\"no matching %s variant\")}", name)
	} else {
		g.imports["encoding/json"] = "json"
		g.line(
			"obj,err:=contract.Decode[map[string]json.RawMessage](data); if "+
				"err!=nil{return nil,err}; tag,err:=contract.Decode[%s](obj[%s]); if "+
				"err!=nil{return nil,fmt.Errorf(%s,err)}; switch tag {",
			g.unionKind(d),
			quote(tag),
			quote(tag+": %w"),
		)
		for _, v := range g.variants(d.key) {
			_, value, _ := g.variantWire(v)
			g.line(
				"case %s: value,err:=contract.Decode[%s](data); if err!=nil{return nil,err}; return &value,nil",
				tagLiteral(value),
				v.name,
			)
		}
		verb := "%q"
		if g.unionKind(d) == "bool" {
			verb = "%v"
		}
		g.line("}; return nil,fmt.Errorf(\"unknown %s tag %s\",tag) }", name, verb)
	}
}

func (g *generator) emitJSONCodec(d *definition, st *types.Struct, isStruct bool, tag, variant, kind string) {
	name := d.name
	if isStruct {
		g.imports["encoding/json"] = "json"
	}
	if g.adjacentUnion(d) != nil {
		g.emitAdjacentVariant(d, st, false)
		return
	}
	g.line("func(v *%s) UnmarshalJSON(data []byte)error{", name)
	if !isStruct {
		g.line("value,err:=%s(data); if err!=nil{return err}", g.integerDecoder(d.typ.Underlying(), d.marks, false))
		g.normalizeText(d, "value", "return err")
		g.line("next:=%s(value); if err:=next.Validate();err!=nil{return err}; *v=next; return nil}", name)
		g.line(
			"func(v %s) MarshalJSON()([]byte,error){if err:=v.Validate();"+
				"err!=nil{return nil,err};return contract.EncodeJSON(%s(v))}",
			name,
			g.typeName(d.typ.Underlying()),
		)
		return
	}
	g.emitJSONObjectDecode(d, st, tag, variant, kind)
	g.line("if err:=next.Validate();err!=nil{return err}; *v=next;return nil}")
	g.emitJSONFields(d, st, tag, variant)
}

// collectPackages keeps dependency traversal order and stops after the first load diagnostic.
func (g *generator) collectPackages(pkgs []*packages.Package) ([]*packages.Package, error) {
	var collected []*packages.Package
	var loadErr error
	// go/packages exposes only TypeError, not go/types' private unused-import
	// code. Stripping bodies can produce either spelling for a valid import.
	unusedImport := regexp.MustCompile(`^"[^"]+" imported (as [^ ]+ )?and not used$`)
	packages.Visit(pkgs, nil, func(p *packages.Package) {
		if loadErr != nil {
			return
		}
		if !g.collectPackage(p, unusedImport, &loadErr) {
			return
		}
		collected = append(collected, p)
	})
	if loadErr != nil {
		return nil, loadErr
	}

	return collected, nil
}
