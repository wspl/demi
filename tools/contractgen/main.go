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
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"

	"golang.org/x/tools/go/packages"
)

type definition struct {
	base     string
	position string
	name     string
	key      string
	typ      *types.Named
	marks    map[string]string
	fields   map[string]map[string]string
}

type generator struct {
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

// generate loads declarations without function bodies so generation also works
// before generated methods exist. Type errors in declarations remain fatal.
func generate(ctx context.Context, patterns []string, ts bool, tsDir string, verify bool) error {
	config := &packages.Config{Context: ctx, Mode: packages.NeedName | packages.NeedFiles | packages.NeedSyntax | packages.NeedTypes | packages.NeedTypesInfo | packages.NeedImports | packages.NeedDeps,
		ParseFile: func(fset *token.FileSet, filename string, src []byte) (*ast.File, error) {
			file, err := parser.ParseFile(fset, filename, src, parser.ParseComments)
			if err != nil {
				return nil, err
			}
			if filepath.Base(filename) == "contract_gen.go" {
				file.Decls = nil
				file.Imports = nil
				return file, nil
			}
			for _, decl := range file.Decls {
				if fn, ok := decl.(*ast.FuncDecl); ok {
					fn.Body = nil
				}
			}
			return file, nil
		}}
	pkgs, err := packages.Load(config, patterns...)
	if err != nil {
		return err
	}
	g := generator{defs: map[string]*definition{}, tsDir: tsDir}
	var collected []*packages.Package
	var loadErr error
	packages.Visit(pkgs, nil, func(p *packages.Package) {
		if loadErr != nil {
			return
		}
		var defs []*definition
		marked := false
		for _, file := range p.Syntax {
			for _, decl := range file.Decls {
				general, ok := decl.(*ast.GenDecl)
				if !ok {
					continue
				}
				for _, spec := range general.Specs {
					if value, ok := spec.(*ast.ValueSpec); ok {
						doc := general.Doc
						if value.Doc != nil {
							doc = value.Doc
						}
						marks := markers(doc)
						if len(marks) > 0 {
							marked = true
							if general.Tok != token.VAR {
								loadErr = fmt.Errorf("%s: %s: table requires a variable", p.Fset.Position(value.Pos()), value.Names[0].Name)
								return
							}
							table, err := readTable(p, value, marks)
							if err != nil {
								loadErr = err
								return
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
						marked = true
					}
					obj := p.TypesInfo.Defs[spec.Name]
					if obj == nil {
						continue
					}
					named, ok := obj.Type().(*types.Named)
					if !ok {
						if len(marks) > 0 {
							loadErr = fmt.Errorf("%s: %s: aliases unsupported", p.Fset.Position(spec.Pos()), spec.Name.Name)
						}
						continue
					}
					d := &definition{position: p.Fset.Position(spec.Pos()).String(), name: spec.Name.Name, key: typeKey(named), typ: named, marks: marks, fields: map[string]map[string]string{}}
					if source, ok := p.TypesInfo.TypeOf(spec.Type).(*types.Named); ok {
						d.base = typeKey(source.Origin())
					}
					if st, ok := spec.Type.(*ast.StructType); ok {
						for _, field := range st.Fields.List {
							if len(field.Names) == 0 && len(markers(field.Doc)) > 0 {
								loadErr = fmt.Errorf("%s: %s: markers on embedded fields are unsupported", p.Fset.Position(field.Pos()), d.name)
							}
							for _, name := range field.Names {
								d.fields[name.Name] = markers(field.Doc)
								if len(d.fields[name.Name]) > 0 {
									marked = true
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
					defs = append(defs, d)
				}
			}
		}
		if !marked {
			return
		}
		for _, problem := range p.Errors {
			if problem.Kind != packages.TypeError || !strings.Contains(problem.Msg, "imported and not used") {
				loadErr = fmt.Errorf("%s", problem)
				return
			}
		}
		for _, d := range defs {
			g.defs[d.key] = d
			g.order = append(g.order, d.key)
		}
		collected = append(collected, p)
	})
	if loadErr != nil {
		return loadErr
	}
	if len(g.order) == 0 && len(g.tables) == 0 {
		return nil
	}
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
	if err := g.normalizeVariants(); err != nil {
		return err
	}
	if err := g.check(collected[0]); err != nil {
		return err
	}
	jsonSchemas, err := g.jsonSchemas()
	if err != nil {
		return err
	}
	g.received = map[string]bool{}
	for _, key := range g.order {
		if has(g.defs[key].marks, "msgpack") {
			g.markReceived(key)
		}
	}
	for key := range g.received {
		if !has(g.defs[key].marks, "msgpack") {
			g.defs[key].marks["msgpack"] = ""
		}
		d := g.defs[key]
		if err := checkMsgpackShape(d.typ.Underlying()); err != nil {
			return fmt.Errorf("%s: %s: %w", d.position, d.name, err)
		}
	}
	sources, err := g.tsSources()
	if err != nil {
		return err
	}
	if ts {
		return g.writeTS(collected[0], sources, verify)
	}
	for _, p := range pkgs {
		g.current = p.Types
		g.imports = map[string]string{}
		g.goCode.Reset()
		for _, name := range g.order {
			d := g.defs[name]
			if d.typ.Obj().Pkg() == p.Types {
				g.emitGo(d)
				if value, ok := jsonSchemas[d.key]; ok {
					g.line("func %sJSONSchema() json.RawMessage { return json.RawMessage(%s) }", d.name, q(string(value)))
				}
			}
		}
		for _, name := range g.order {
			d := g.defs[name]
			if d.typ.Obj().Pkg() == p.Types && has(d.marks, "msgpack") {
				g.emitMsgpack(d)
			}
		}
		if g.goCode.Len() == 0 {
			continue
		}
		var header strings.Builder
		fmt.Fprintf(&header, "// Code generated by contractgen; DO NOT EDIT.\npackage %s\nimport (\"fmt\"; \"encoding/json\"; \"github.com/wspl/demi/internal/contract\";", p.Name)
		paths := make([]string, 0, len(g.imports))
		for path := range g.imports {
			paths = append(paths, path)
		}
		sort.Strings(paths)
		for _, path := range paths {
			fmt.Fprintf(&header, "%s %s;", g.imports[path], q(path))
		}
		header.WriteString(")\n")
		code, err := format.Source(append([]byte(header.String()), g.goCode.Bytes()...))
		if err != nil {
			return fmt.Errorf("format generated code: %w", err)
		}
		if err := writeGenerated(filepath.Join(filepath.Dir(p.GoFiles[0]), "contract_gen.go"), code, verify); err != nil {
			return err
		}
	}
	return nil
}

// normalizeVariants resolves a tag-only marker against its sealing interface.
func (g *generator) normalizeVariants() error {
	for _, name := range g.order {
		d := g.defs[name]
		parts := strings.Fields(d.marks["variant"])
		if has(d.marks, "variant") && len(parts) != 1 && len(parts) != 2 {
			return fmt.Errorf("%s: %s: variant requires a tag", d.position, d.name)
		}
		if len(parts) == 2 {
			d.marks["variant"] = d.typ.Obj().Pkg().Path() + "." + parts[0] + " " + parts[1]
			continue
		}
		if len(parts) != 1 {
			continue
		}
		var unions []string
		for _, other := range g.order {
			u := g.defs[other]
			if !has(u.marks, "union") || u.typ.Obj().Pkg() != d.typ.Obj().Pkg() {
				continue
			}
			iface, ok := u.typ.Underlying().(*types.Interface)
			if !ok {
				continue
			}
			if types.Implements(types.NewPointer(d.typ), iface) {
				unions = append(unions, other)
			}
		}
		if len(unions) == 0 {
			for _, other := range g.order {
				u := g.defs[other]
				if has(u.marks, "union") && u.typ.Obj().Pkg() == d.typ.Obj().Pkg() {
					unions = append(unions, other)
				}
			}
		}
		if len(unions) != 1 {
			return fmt.Errorf("%s: variant %s requires one sealing interface or explicit <Union> <tag>", d.position, d.name)
		}
		d.marks["variant"] = unions[0] + " " + parts[0]
	}
	return nil
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
		case "union", "variant", "nullable", "length", "range", "enum", "pattern", "timestamp", "check", "id", "base64", "msgpack", "strict", "tolerant", "root", "format", "table", "schema":
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
func q(s string) string                           { return strconv.Quote(s) }
func typeKey(t *types.Named) string               { return t.Obj().Pkg().Path() + "." + t.Obj().Name() }
func (g *generator) typeName(t types.Type) string { return types.TypeString(t, g.qualifier) }
func (g *generator) qualifier(p *types.Package) string {
	if p != nil && p.Path() == "encoding/json" {
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
func schema(name string) string                 { return strings.ToLower(name[:1]) + name[1:] + "Schema" }
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
func (g *generator) variants(name string) []*definition {
	var out []*definition
	for _, n := range g.order {
		d := g.defs[n]
		if strings.HasPrefix(d.marks["variant"], name+" ") {
			out = append(out, d)
		}
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
		return "func(b []byte)(" + g.typeName(t) + ",error){ return contract.Record(b," + g.decoder(t.Elem()) + "," + strconv.FormatBool(isPointer(t.Elem())) + ") }"
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return "contract.Bytes"
		}
		return "func(b []byte)(" + g.typeName(t) + ",error){ return contract.List(b," + g.decoder(t.Elem()) + ") }"
	case *types.Named:
		if _, ok := t.Underlying().(*types.Interface); ok {
			return g.prefix(t) + "Decode" + t.Obj().Name()
		}
	}
	return "contract.Decode[" + g.typeName(t) + "]"
}

func (g *generator) emitGo(d *definition) {
	name := d.name
	if has(d.marks, "union") {
		tag := strings.TrimPrefix(d.marks["union"], "tag=")
		g.line("func Decode%s(data []byte)(%s,error) {", name, name)
		g.line("obj,err:=contract.Decode[map[string]json.RawMessage](data); if err!=nil{return nil,err}; tag,err:=contract.Decode[string](obj[%s]); if err!=nil{return nil,fmt.Errorf(%s,err)}; switch tag {", q(tag), q(tag+": %w"))
		for _, v := range g.variants(d.key) {
			_, value, _ := strings.Cut(v.marks["variant"], " ")
			g.line("case %s: value,err:=contract.Decode[%s](data); if err!=nil{return nil,err}; return &value,nil", q(value), v.name)
		}
		g.line("}; return nil,fmt.Errorf(\"unknown %s tag %%q\",tag) }", name)
		// An interface cannot own UnmarshalJSON; the transport holder does.
		g.line("type %sJSON struct { Value %s }; func(v *%sJSON) UnmarshalJSON(data []byte) error { value,err:=Decode%s(data); if err==nil {v.Value=value}; return err }; func(v %sJSON) MarshalJSON()([]byte,error){ if err:=Validate%s(v.Value);err!=nil{return nil,err}; return json.Marshal(v.Value) }", name, name, name, name, name, name)
		g.line("func Validate%s(value %s)error{return contractValidate%s(value,0)}", name, name, name)
		g.line("func contractValidate%s(value %s,depth int)error{if depth>1000{return fmt.Errorf(\"validation nesting exceeds 1000\")};switch v:=value.(type){", name, name)
		for _, v := range g.variants(d.key) {
			g.line("case *%s: if v==nil{return fmt.Errorf(\"nil variant\")}; return contractValidate%s(*v,depth+1)", v.name, v.name)
		}
		g.line("default:return fmt.Errorf(\"nil or unsupported %s\")}}", name)
		return
	}
	if has(d.marks, "id") {
		g.line("func Parse%s(value string)(%s,error){", name, name)
		g.normalizeText(d, "value", "return \"\",err")
		g.line("v:=%s(value);if err:=v.Validate();err!=nil{return \"\",err};return v,nil}", name)
	}
	st, isStruct := g.object(d)
	union, variant, _ := strings.Cut(d.marks["variant"], " ")
	tag := ""
	if union != "" {
		tag = strings.TrimPrefix(g.defs[union].marks["union"], "tag=")
		method := g.defs[union].typ.Underlying().(*types.Interface).Method(0).Name()
		if obj, _, _ := types.LookupFieldOrMethod(types.NewPointer(d.typ), true, d.typ.Obj().Pkg(), method); obj == nil {
			g.line("func (*%s) %s() {}", name, method)
		}
	}
	g.line("func Decode%s(data []byte)(%s,error){return contract.Decode[%s](data)}", name, name, name)
	g.line("func(v %s)Validate()error{return contractValidate%s(v,0)}", name, name)
	g.line("func contractValidate%s(v %s,depth int)error{if depth>1000{return fmt.Errorf(\"validation nesting exceeds 1000\")}", name, name)
	if isStruct {
		validationStruct := d.typ.Underlying().(*types.Struct)
		for i := 0; i < validationStruct.NumFields(); i++ {
			f := validationStruct.Field(i)
			key := strings.Split(reflect.StructTag(validationStruct.Tag(i)).Get("json"), ",")[0]
			m := d.fields[f.Name()]
			if len(strings.Split(reflect.StructTag(validationStruct.Tag(i)).Get("json"), ",")) > 1 {
				m["optional"] = ""
			}
			g.validation(f.Type(), "v."+f.Name(), q(key), m)
		}
	} else {
		g.validation(d.typ.Underlying(), "v", q(""), d.marks)
	}
	if custom := d.marks["check"]; custom != "" {
		g.line("if err:=%s(v);err!=nil{return err}", custom)
	}
	g.line("return nil }")
	g.line("func(v *%s) UnmarshalJSON(data []byte)error{", name)
	if !isStruct {
		g.line("value,err:=%s(data); if err!=nil{return err}", g.decoder(d.typ.Underlying()))
		g.normalizeText(d, "value", "return err")
		g.line("next:=%s(value); if err:=next.Validate();err!=nil{return err}; *v=next; return nil}", name)
		g.line("func(v %s) MarshalJSON()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};return json.Marshal(%s(v))}", name, g.typeName(d.typ.Underlying()))
		return
	}
	g.line("obj,err:=contract.Decode[map[string]json.RawMessage](data); if err!=nil{return err}; var next %s", name)
	if !has(d.marks, "tolerant") {
		g.line("for key:=range obj{switch key{")
		keys := []string{}
		for i := 0; i < st.NumFields(); i++ {
			keys = append(keys, q(strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")[0]))
		}
		if tag != "" {
			keys = append(keys, q(tag))
		}
		if len(keys) > 0 {
			g.line("case %s:", strings.Join(keys, ","))
		}
		g.line("default:return contract.At(key,fmt.Errorf(\"unknown field\"))}}")
	}
	if tag != "" {
		g.line("if raw,ok:=obj[%s];!ok { return fmt.Errorf(\"missing union tag\") } else { value,err:=contract.Decode[string](raw);if err!=nil||value!=%s{return fmt.Errorf(\"invalid union tag\")} }", q(tag), q(variant))
	}
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
		key := opts[0]
		optional := len(opts) > 1
		nullable := has(d.fields[f.Name()], "nullable")
		g.line("{raw,ok:=obj[%s];", q(key))
		if !optional {
			g.line("if !ok{return contract.At(%s,fmt.Errorf(\"required field is absent\"))}", q(key))
		}
		g.line("if ok {")
		if nullable {
			g.line("if !contract.IsNull(raw) {")
		}
		g.line("value,err:=%s(raw); if err!=nil{return contract.At(%s,err)}; next.%s=value", g.decoder(f.Type()), q(key), f.Name())
		if nullable {
			g.line("}")
		}
		g.line("}}")
	}
	g.line("if err:=next.Validate();err!=nil{return err}; *v=next;return nil}")
	g.emitJSONFields(d, st, tag, variant)
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
		g.validation(ptr.Elem(), "(*"+expr+")", path, m)
		g.line("}")
		return
	}
	if named, ok := t.(*types.Named); ok {
		if _, ok := named.Underlying().(*types.Interface); ok {
			if has(m, "nullable") {
				g.line("if %s!=nil{", expr)
			}
			if named.Obj().Pkg() == g.current {
				g.line("if err:=contractValidate%s(%s,depth+1);err!=nil{return contract.At(%s,err)}", named.Obj().Name(), expr, path)
			} else {
				g.line("if err:=%sValidate%s(%s);err!=nil{return contract.At(%s,err)}", g.prefix(named), named.Obj().Name(), expr, path)
			}
			if has(m, "nullable") {
				g.line("}")
			}
			return
		}
		if named.Obj().Pkg() == g.current {
			g.line("if err:=contractValidate%s(%s,depth+1);err!=nil{return contract.At(%s,err)}", named.Obj().Name(), expr, path)
		} else {
			g.line("if err:=%s.Validate();err!=nil{return contract.At(%s,err)}", expr, path)
		}
	}
	if record, ok := t.(*types.Map); ok {
		g.line("if %s==nil{return contract.At(%s,fmt.Errorf(\"required record is nil\"))}", expr, path)
		g.line("for key,item:=range %s{_=item;if err:=contract.Text(key,0,-1,\"\");err!=nil{return contract.At(%s,err)}", expr, path)
		g.validation(record.Elem(), "item", "fmt.Sprintf(\"%s[%q]\","+path+",key)", map[string]string{"nullable": ""})
		g.line("}")
	}
	if slice, ok := t.(*types.Slice); ok {
		g.line("if %s==nil{return contract.At(%s,fmt.Errorf(\"required array is nil\"))}", expr, path)
		g.line("for i,item:=range %s { _=i;_=item", expr)
		g.validation(slice.Elem(), "item", "fmt.Sprintf(\"%s[%d]\","+path+",i)", map[string]string{})
		g.line("}")
	}
	g.rules(t, expr, path, m)
}
func (g *generator) rules(t types.Type, expr, path string, m map[string]string) {
	if b, ok := t.Underlying().(*types.Basic); ok && b.Info()&types.IsFloat != 0 {
		g.imports["math"] = "math"
		g.line("if math.IsNaN(float64(%s)) || math.IsInf(float64(%s),0){return contract.At(%s,fmt.Errorf(\"number must be finite\"))}", expr, expr, path)
	}
	if format := m["format"]; format != "" {
		if format == "trimmed" {
			g.line("if contract.Trim(string(%s))!=string(%s){return contract.At(%s,fmt.Errorf(\"text is not trimmed\"))}", expr, expr, path)
		} else {
			helper := "Email"
			if format == "http-url" {
				helper = "HTTPURL"
			}
			g.line("if value,err:=contract.%s(string(%s));err!=nil{return contract.At(%s,err)}else if value!=string(%s){return contract.At(%s,fmt.Errorf(\"text is not canonical\"))}", helper, expr, path, expr, path)
		}
	}
	if has(m, "base64") {
		g.line("if err:=contract.Base64(string(%s));err!=nil{return contract.At(%s,err)}", expr, path)
	}
	if has(m, "timestamp") {
		g.line("if err:=contract.Timestamp(string(%s));err!=nil{return contract.At(%s,err)}", expr, path)
	}
	if values := m["enum"]; values != "" {
		var vals []string
		for _, v := range strings.Fields(values) {
			vals = append(vals, q(v))
		}
		g.line("switch string(%s){case %s:default:return contract.At(%s,fmt.Errorf(\"unknown value\"))}", expr, strings.Join(vals, ","), path)
	}
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
		g.line("if err:=contract.Text(string(%s),%s,%s,%s);err!=nil{return contract.At(%s,err)}", expr, minimum, maximum, q(m["pattern"]), path)
	} else {
		if minimum := b["min"]; minimum != "" {
			g.line("if len(%s)<%s{return contract.At(%s,fmt.Errorf(\"too few items\"))}", expr, minimum, path)
		}
		if maximum := b["max"]; maximum != "" {
			g.line("if len(%s)>%s{return contract.At(%s,fmt.Errorf(\"too many items\"))}", expr, maximum, path)
		}
	}
	for _, key := range []string{"min", "max"} {
		if bound := bounds(m["range"])[key]; bound != "" {
			op := "<"
			if key == "max" {
				op = ">"
			}
			g.line("if %s%s%s{return contract.At(%s,fmt.Errorf(\"outside numeric bounds\"))}", expr, op, bound, path)
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
	return os.WriteFile(path, code, 0644)
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
		}
	}
}
