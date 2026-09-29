package wiregen

import (
	"bytes"
	"errors"
	"fmt"
	"go/ast"
	"go/build"
	"go/constant"
	"go/parser"
	"go/printer"
	"go/token"
	"go/types"
	"os"
	pathpkg "path"
	"path/filepath"
	"reflect"
	"slices"
	"strconv"
	"strings"
)

// The directives that mark declarations.
const (
	directiveWire    = "//demi:wire"
	directiveUnion   = "//demi:union"
	directiveVariant = "//demi:variant"
	directiveOpaque  = "//demi:opaque"
	// open adds to a wire struct or a variant: it ignores members it does not
	// have.
	directiveOpen = "//demi:open"
)

// generatedSuffix ends the name of every file the generator writes.
const generatedSuffix = "_wire.go"

// Load reads the wire declarations of the Go package in dir: the files of the
// package for the host system, without its tests and without the files the
// generator wrote.
func Load(dir string) (*Package, error) {
	built, err := build.Default.ImportDir(dir, 0)
	if err != nil {
		var none *build.NoGoError
		if !errors.As(err, &none) {
			return nil, err
		}
	}
	if err := refuseConstrainedDeclarations(dir, built); err != nil {
		return nil, err
	}
	sources := map[string][]byte{}
	for _, name := range built.GoFiles {
		if strings.HasSuffix(name, generatedSuffix) {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, name))
		if err != nil {
			return nil, err
		}
		sources[name] = data
	}
	return LoadSource(sources)
}

// refuseConstrainedDeclarations refuses a file that declares wire types and
// builds on some platforms only, by its name or by a build line: a wire type is
// the same on every platform, and generated code is written once.
func refuseConstrainedDeclarations(dir string, built *build.Package) error {
	for _, name := range slices.Concat(built.GoFiles, built.IgnoredGoFiles) {
		if strings.HasSuffix(name, generatedSuffix) || strings.HasSuffix(name, "_test.go") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, name))
		if err != nil {
			return err
		}
		if !bytes.Contains(data, []byte("//demi:")) {
			continue
		}
		if slices.Contains(built.IgnoredGoFiles, name) || buildsDifferently(dir, name) {
			return fmt.Errorf("%s: a file with wire declarations has build constraints; a wire type is the same on every platform", filepath.Join(dir, name))
		}
	}
	return nil
}

// buildsDifferently reports whether a file is part of the package on some of
// the platforms and not on others.
func buildsDifferently(dir, name string) bool {
	var first bool
	for i, platform := range [][2]string{{"linux", "amd64"}, {"windows", "arm64"}, {"darwin", "arm64"}, {"linux", "riscv64"}} {
		context := build.Default
		context.GOOS, context.GOARCH = platform[0], platform[1]
		match, err := context.MatchFile(dir, name)
		if err != nil {
			return true
		}
		if i == 0 {
			first = match
		} else if match != first {
			return true
		}
	}
	return false
}

// LoadSource reads the wire declarations of the package that the source files,
// by name, make up.
func LoadSource(sources map[string][]byte) (*Package, error) {
	fset := token.NewFileSet()
	var names []string
	for name := range sources {
		names = append(names, name)
	}
	slices.Sort(names)
	var files []*ast.File
	for _, name := range names {
		file, err := parser.ParseFile(fset, name, sources[name], parser.ParseComments|parser.SkipObjectResolution)
		if err != nil {
			return nil, err
		}
		files = append(files, file)
	}
	p := &reader{
		fset:    fset,
		files:   files,
		methods: map[string]map[string]bool{},
		pkg: &Package{
			Structs:  map[string]*Struct{},
			Unions:   map[string]*Union{},
			Opaque:   map[string]bool{},
			named:    map[string]*Type{},
			patterns: map[string]string{},
		},
		specs:   map[string]*ast.StructType{},
		imports: map[string]map[string]string{},
		used:    map[string]map[string]string{},
	}
	for _, file := range files {
		p.imports[filepath.Base(fset.Position(file.Pos()).Filename)] = importsOf(file)
	}
	if len(files) > 0 {
		p.pkg.Name = files[0].Name.Name
	}
	if err := p.load(); err != nil {
		return nil, err
	}
	return p.pkg, nil
}

// A reader is the state of reading one package.
type reader struct {
	fset  *token.FileSet
	files []*ast.File
	pkg   *Package
	// methods are the names of the methods of each type.
	methods map[string]map[string]bool
	// specs are the struct declarations of the marked structs.
	specs map[string]*ast.StructType
	// decls are the declarations to resolve once every name is known.
	decls []marked
	// imports are the packages each file imports, by the name the file gives
	// them, and used those of them that its wire types name; file is the file
	// being resolved.
	imports map[string]map[string]string
	used    map[string]map[string]string
	file    string
}

// importsOf returns the packages a file imports by the name it gives them.
func importsOf(file *ast.File) map[string]string {
	imports := map[string]string{}
	for _, spec := range file.Imports {
		path, err := strconv.Unquote(spec.Path.Value)
		if err != nil {
			continue
		}
		name := pathpkg.Base(path)
		if spec.Name != nil {
			name = spec.Name.Name
		}
		imports[name] = path
	}
	return imports
}

// use records that the wire types of the file being resolved name the package
// imported as qualifier, and returns whether the file imports one.
func (p *reader) use(qualifier string) bool {
	path, ok := p.imports[p.file][qualifier]
	if !ok {
		return false
	}
	if p.used[p.file] == nil {
		p.used[p.file] = map[string]string{}
	}
	p.used[p.file][qualifier] = path
	return true
}

// A marked is a declaration with a directive, and the file it is in.
type marked struct {
	file string
	name string
	spec *ast.TypeSpec
	doc  string
	// directives are the lines of the declaration's comment that start with
	// //demi:.
	directives []string
}

func (p *reader) errorf(pos token.Pos, format string, args ...any) error {
	return fmt.Errorf("%s: %s", p.fset.Position(pos), fmt.Sprintf(format, args...))
}

func (p *reader) load() error {
	p.constants()
	byFile := map[string]*File{}
	for _, file := range p.files {
		fileName := filepath.Base(p.fset.Position(file.Pos()).Filename)
		for _, decl := range file.Decls {
			switch decl := decl.(type) {
			case *ast.FuncDecl:
				p.method(decl)
			case *ast.GenDecl:
				p.patternsOf(decl)
				if decl.Tok != token.TYPE {
					continue
				}
				for _, spec := range decl.Specs {
					spec := spec.(*ast.TypeSpec)
					doc := spec.Doc
					if doc == nil && len(decl.Specs) == 1 {
						doc = decl.Doc
					}
					m := marked{file: fileName, name: spec.Name.Name, spec: spec}
					if doc != nil {
						m.doc = strings.TrimSpace(doc.Text())
						for _, comment := range doc.List {
							if strings.HasPrefix(comment.Text, "//demi:") {
								m.directives = append(m.directives, comment.Text)
							}
						}
					}
					if err := p.declare(&m, byFile); err != nil {
						return err
					}
				}
			}
		}
	}
	for _, m := range p.decls {
		if err := p.resolve(m); err != nil {
			return err
		}
	}
	if err := p.link(); err != nil {
		return err
	}
	for _, file := range p.files {
		name := filepath.Base(p.fset.Position(file.Pos()).Filename)
		if f, ok := byFile[name]; ok {
			f.Imports = p.used[name]
			p.pkg.Files = append(p.pkg.Files, f)
		}
	}
	return nil
}

// declare records what a type declaration is, before any field is resolved.
func (p *reader) declare(m *marked, byFile map[string]*File) error {
	kinds, open := 0, false
	for _, directive := range m.directives {
		switch fields := strings.Fields(directive); fields[0] {
		case directiveWire, directiveUnion, directiveVariant, directiveOpaque:
			kinds++
		case directiveOpen:
			if len(fields) > 1 {
				return p.errorf(m.spec.Pos(), "%s: %s takes no argument", m.name, directiveOpen)
			}
			open = true
		default:
			return p.errorf(m.spec.Pos(), "%s: unknown directive %s", m.name, fields[0])
		}
	}
	if kinds > 1 {
		return p.errorf(m.spec.Pos(), "%s: a declaration has one directive", m.name)
	}
	if open {
		directive := ""
		if kinds == 1 {
			directive = strings.Fields(m.directives[kindIndex(m.directives)])[0]
		}
		if directive != directiveWire && directive != directiveVariant {
			return p.errorf(m.spec.Pos(), "%s: %s marks a wire struct or a variant", m.name, directiveOpen)
		}
	}
	if kinds == 0 {
		// A type named after a basic type is not a wire type, but its fields
		// name it.
		if ident, ok := m.spec.Type.(*ast.Ident); ok && !m.spec.Assign.IsValid() {
			if t := basicType(ident.Name); t != nil {
				named := *t
				named.Name = m.name
				named.Src = m.name
				p.pkg.named[m.name] = &named
			}
		}
		return nil
	}
	directive := strings.Fields(m.directives[kindIndex(m.directives)])
	switch directive[0] {
	case directiveOpaque:
		if _, ok := m.spec.Type.(*ast.StructType); !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks a struct", m.name, directive[0])
		}
		// An opaque type has no generated code, so it has no file to write.
		p.pkg.Opaque[m.name] = true
		return nil
	case directiveWire, directiveVariant:
		if _, ok := m.spec.Type.(*ast.StructType); !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks a struct", m.name, directive[0])
		}
		s := &Struct{Name: m.name, Doc: m.doc, Open: open}
		if directive[0] == directiveVariant && len(directive) > 1 {
			s.Tag = directive[1]
		}
		p.pkg.Structs[m.name] = s
		p.specs[m.name] = m.spec.Type.(*ast.StructType)
	case directiveUnion:
		iface, ok := m.spec.Type.(*ast.InterfaceType)
		if !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks an interface", m.name, directive[0])
		}
		u := &Union{Name: m.name, Doc: m.doc}
		switch {
		case len(directive) == 2 && directive[1] == "untagged":
		case len(directive) == 2 && strings.HasPrefix(directive[1], "tag="):
			u.TagName = strings.TrimPrefix(directive[1], "tag=")
		case len(directive) == 3 && strings.HasPrefix(directive[1], "tag=") && strings.HasPrefix(directive[2], "content="):
			u.TagName = strings.TrimPrefix(directive[1], "tag=")
			u.ContentName = strings.TrimPrefix(directive[2], "content=")
			if u.TagName == "" || u.ContentName == "" || u.TagName == u.ContentName {
				return p.errorf(m.spec.Pos(), "%s: an adjacently tagged union names its tag and its content, apart", m.name)
			}
		default:
			return p.errorf(m.spec.Pos(), "%s: a union is `tag=NAME` or `untagged`, and `tag=NAME` may be followed by `content=NAME`", m.name)
		}
		if len(iface.Methods.List) != 1 || len(iface.Methods.List[0].Names) != 1 || ast.IsExported(iface.Methods.List[0].Names[0].Name) {
			return p.errorf(m.spec.Pos(), "%s: a union has one unexported method", m.name)
		}
		u.Sealed = iface.Methods.List[0].Names[0].Name
		p.pkg.Unions[m.name] = u
	}
	f := byFile[m.file]
	if f == nil {
		f = &File{Name: m.file}
		byFile[m.file] = f
	}
	f.Types = append(f.Types, m.name)
	p.decls = append(p.decls, *m)
	return nil
}

// kindIndex returns the index of the directive that says what a declaration is;
// //demi:open only adds to it.
func kindIndex(directives []string) int {
	for i, directive := range directives {
		if strings.Fields(directive)[0] != directiveOpen {
			return i
		}
	}
	return 0
}

func basicType(name string) *Type {
	switch name {
	case "string":
		return &Type{Kind: KindString, Src: name}
	case "bool":
		return &Type{Kind: KindBool, Src: name}
	case "int":
		return &Type{Kind: KindInt, Src: name}
	case "int8":
		return &Type{Kind: KindInt, Src: name, Bits: 8}
	case "int16":
		return &Type{Kind: KindInt, Src: name, Bits: 16}
	case "int32":
		return &Type{Kind: KindInt, Src: name, Bits: 32}
	case "int64":
		return &Type{Kind: KindInt, Src: name, Bits: 64}
	case "uint":
		return &Type{Kind: KindUint, Src: name}
	case "uint8":
		return &Type{Kind: KindUint, Src: name, Bits: 8}
	case "uint16":
		return &Type{Kind: KindUint, Src: name, Bits: 16}
	case "uint32":
		return &Type{Kind: KindUint, Src: name, Bits: 32}
	case "uint64":
		return &Type{Kind: KindUint, Src: name, Bits: 64}
	}
	return nil
}

// method records the name of a method and the type it belongs to.
func (p *reader) method(decl *ast.FuncDecl) {
	if decl.Recv == nil || len(decl.Recv.List) != 1 {
		return
	}
	expr := decl.Recv.List[0].Type
	if star, ok := expr.(*ast.StarExpr); ok {
		expr = star.X
	}
	ident, ok := expr.(*ast.Ident)
	if !ok {
		return
	}
	if p.methods[ident.Name] == nil {
		p.methods[ident.Name] = map[string]bool{}
	}
	p.methods[ident.Name][decl.Name.Name] = true
}

// patternsOf records the source of the regular expressions a declaration
// assigns to variables: `var name = regexp.MustCompile("...")`.
func (p *reader) patternsOf(decl *ast.GenDecl) {
	if decl.Tok != token.VAR {
		return
	}
	for _, spec := range decl.Specs {
		spec := spec.(*ast.ValueSpec)
		if len(spec.Names) != len(spec.Values) {
			continue
		}
		for i, value := range spec.Values {
			call, ok := value.(*ast.CallExpr)
			if !ok || len(call.Args) != 1 {
				continue
			}
			selector, ok := call.Fun.(*ast.SelectorExpr)
			if !ok || selector.Sel.Name != "MustCompile" {
				continue
			}
			literal, ok := call.Args[0].(*ast.BasicLit)
			if !ok || literal.Kind != token.STRING {
				continue
			}
			source, err := strconv.Unquote(literal.Value)
			if err != nil {
				continue
			}
			p.pkg.patterns[spec.Names[i].Name] = source
		}
	}
}

// emptyImporter imports every package as an empty one, so that checking a
// package for its constants needs no other package.
type emptyImporter struct{}

func (emptyImporter) Import(path string) (*types.Package, error) {
	pkg := types.NewPackage(path, filepath.Base(path))
	pkg.MarkComplete()
	return pkg, nil
}

// constants evaluates the package's constants. The package is checked without
// its imports, whose errors do not matter here.
func (p *reader) constants() {
	config := types.Config{
		Importer: emptyImporter{},
		Error:    func(error) {},
	}
	checked, _ := config.Check(p.pkg.Name, p.fset, p.files, nil)
	p.pkg.consts = map[string]constant.Value{}
	if checked == nil {
		return
	}
	for _, name := range checked.Scope().Names() {
		if c, ok := checked.Scope().Lookup(name).(*types.Const); ok {
			p.pkg.consts[name] = c.Val()
		}
	}
}

// resolve reads the fields of a marked struct, and the members a union's
// interface has.
func (p *reader) resolve(m marked) error {
	s, ok := p.pkg.Structs[m.name]
	if !ok {
		return nil
	}
	spec := p.specs[m.name]
	p.file = m.file
	s.Check = p.methods[m.name]["check"]
	for _, field := range spec.Fields.List {
		if len(field.Names) != 1 {
			return p.errorf(field.Pos(), "%s: a wire struct declares one field per line, and embeds nothing", m.name)
		}
		name := field.Names[0].Name
		where := m.name + "." + name
		if !ast.IsExported(name) {
			return p.errorf(field.Pos(), "%s: a wire field is exported", where)
		}
		if field.Tag == nil {
			return p.errorf(field.Pos(), "%s: a wire field has a json tag", where)
		}
		tagText, err := strconv.Unquote(field.Tag.Value)
		if err != nil {
			return p.errorf(field.Pos(), "%s: %v", where, err)
		}
		tag := reflect.StructTag(tagText)
		jsonName, options, _ := strings.Cut(tag.Get("json"), ",")
		omitzero, inline := false, false
		for _, option := range strings.Split(options, ",") {
			switch option {
			case "":
			case "omitzero":
				omitzero = true
			case "inline":
				inline = true
			default:
				return p.errorf(field.Pos(), "%s: the json option %q is not one of a wire field's", where, option)
			}
		}
		if jsonName == "-" || jsonName == "" && !inline {
			return p.errorf(field.Pos(), "%s: a wire field has a json name", where)
		}
		if inline && jsonName != "" {
			return p.errorf(field.Pos(), "%s: an inline field has no json name", where)
		}
		t, err := p.typeOf(field.Type)
		if err != nil {
			return p.errorf(field.Pos(), "%s: %v", where, err)
		}
		if inline {
			union := p.pkg.Unions[t.Name]
			switch {
			case t.Kind != KindUnion || union.ContentName == "":
				return p.errorf(field.Pos(), "%s: an inline field is an adjacently tagged union", where)
			case omitzero:
				return p.errorf(field.Pos(), "%s: an inline field is required", where)
			case slices.ContainsFunc(s.Fields, func(f *Field) bool { return f.Inline }):
				return p.errorf(field.Pos(), "%s: a struct has one inline field", m.name)
			}
		}
		var rules []Rule
		if check, ok := tag.Lookup("check"); ok {
			rules, err = p.parseRules(check)
			if err != nil {
				return p.errorf(field.Pos(), "%s: %v", where, err)
			}
		}
		nullable := slices.ContainsFunc(rules, func(rule Rule) bool { return rule.Kind == RuleNullable })
		rules = slices.DeleteFunc(rules, func(rule Rule) bool { return rule.Kind == RuleNullable })
		switch {
		case nullable && (t.Kind != KindPointer || omitzero):
			return p.errorf(field.Pos(), "%s: a nullable field is a required pointer, without omitzero", where)
		case omitzero && t.Kind != KindPointer:
			return p.errorf(field.Pos(), "%s: an optional field is a pointer with omitzero", where)
		case t.Kind == KindPointer && !omitzero && !nullable:
			return p.errorf(field.Pos(), "%s: a pointer field is optional and has omitzero", where)
		}
		if err := p.checkRules(t, rules); err != nil {
			return p.errorf(field.Pos(), "%s: %v", where, err)
		}
		if err := p.foreignRules(t, rules); err != nil {
			return p.errorf(field.Pos(), "%s: %v", where, err)
		}
		f := &Field{
			Name:     name,
			JSON:     jsonName,
			Doc:      strings.TrimSpace(field.Doc.Text()),
			Type:     t,
			Required: !omitzero,
			Nullable: nullable,
			Inline:   inline,
			Rules:    rules,
		}
		s.Fields = append(s.Fields, f)
	}
	return nil
}

// typeOf reads the type of a field.
func (p *reader) typeOf(expr ast.Expr) (*Type, error) {
	src := p.source(expr)
	switch expr := expr.(type) {
	case *ast.Ident:
		if t := basicType(expr.Name); t != nil {
			return t, nil
		}
		if _, ok := p.pkg.Structs[expr.Name]; ok {
			return &Type{Kind: KindStruct, Name: expr.Name, Src: src}, nil
		}
		if p.pkg.Opaque[expr.Name] {
			return &Type{Kind: KindStruct, Name: expr.Name, Src: src, Opaque: true}, nil
		}
		if _, ok := p.pkg.Unions[expr.Name]; ok {
			return &Type{Kind: KindUnion, Name: expr.Name, Src: src}, nil
		}
		if t, ok := p.pkg.named[expr.Name]; ok {
			return t, nil
		}
		return nil, fmt.Errorf("%s is not a type of the wire: mark it with //demi:wire", expr.Name)
	case *ast.StarExpr:
		elem, err := p.typeOf(expr.X)
		if err != nil {
			return nil, err
		}
		if elem.Kind == KindPointer {
			return nil, fmt.Errorf("a pointer to %s", elem.Src)
		}
		return &Type{Kind: KindPointer, Elem: elem, Src: src}, nil
	case *ast.ArrayType:
		if expr.Len != nil {
			return nil, fmt.Errorf("an array %s; use a slice", src)
		}
		elem, err := p.typeOf(expr.Elt)
		if err != nil {
			return nil, err
		}
		if elem.Kind == KindPointer || elem.Kind == KindRaw {
			return nil, fmt.Errorf("a slice of %s", elem.Src)
		}
		return &Type{Kind: KindSlice, Elem: elem, Src: src}, nil
	case *ast.MapType:
		key, err := p.typeOf(expr.Key)
		if err != nil {
			return nil, err
		}
		if key.Kind != KindString {
			return nil, fmt.Errorf("a map with keys of %s; the keys are strings", key.Src)
		}
		elem, err := p.typeOf(expr.Value)
		if err != nil {
			return nil, err
		}
		if elem.Kind == KindPointer || elem.Kind == KindRaw {
			return nil, fmt.Errorf("a map of %s", elem.Src)
		}
		return &Type{Kind: KindMap, Key: key, Elem: elem, Src: src}, nil
	case *ast.SelectorExpr:
		x, ok := expr.X.(*ast.Ident)
		if ok && x.Name == "jsontext" && expr.Sel.Name == "Value" {
			return &Type{Kind: KindRaw, Src: src}, nil
		}
		// A wire struct of another package: it is decoded by its own generated
		// code, so it is opaque here.
		if ok && p.use(x.Name) {
			return &Type{Kind: KindStruct, Name: expr.Sel.Name, Src: src, Opaque: true, Qualifier: x.Name}, nil
		}
	}
	return nil, fmt.Errorf("the type %s is not one the wire holds", src)
}

// source prints an expression as Go source.
func (p *reader) source(expr ast.Expr) string {
	var out bytes.Buffer
	if err := printer.Fprint(&out, p.fset, expr); err != nil {
		return ""
	}
	return out.String()
}

// link finds the variants of each union: the marked structs that have its
// sealed method.
func (p *reader) link() error {
	for _, m := range p.decls {
		s, ok := p.pkg.Structs[m.name]
		if !ok {
			continue
		}
		isVariant := len(m.directives) > 0 && strings.HasPrefix(m.directives[kindIndex(m.directives)], directiveVariant)
		var owner *Union
		for _, u := range p.pkg.Unions {
			if p.methods[m.name][u.Sealed] {
				if owner != nil {
					return p.errorf(m.spec.Pos(), "%s: a variant of two unions", m.name)
				}
				owner = u
			}
		}
		switch {
		case isVariant && owner == nil:
			return p.errorf(m.spec.Pos(), "%s: a variant that implements no union's method", m.name)
		case !isVariant && owner != nil:
			return p.errorf(m.spec.Pos(), "%s: implements the union %s and is not marked //demi:variant", m.name, owner.Name)
		case owner == nil:
			continue
		}
		if owner.TagName != "" && s.Tag == "" {
			return p.errorf(m.spec.Pos(), "%s: a variant of the tagged union %s names its tag: //demi:variant NAME", m.name, owner.Name)
		}
		if owner.TagName == "" && s.Tag != "" {
			return p.errorf(m.spec.Pos(), "%s: a variant of the untagged union %s has no tag", m.name, owner.Name)
		}
		s.Union = owner
		owner.Variants = append(owner.Variants, s)
	}
	for _, u := range p.pkg.Unions {
		if len(u.Variants) == 0 {
			return fmt.Errorf("%s: the union has no variants", u.Name)
		}
		tags := map[string]bool{}
		for _, v := range u.Variants {
			if u.TagName != "" {
				if tags[v.Tag] {
					return fmt.Errorf("%s: two variants of the union %s have the tag %q", v.Name, u.Name, v.Tag)
				}
				tags[v.Tag] = true
				for _, f := range v.Fields {
					// The members of an adjacently tagged variant are in its content,
					// beside no tag.
					if f.JSON == u.TagName && u.ContentName == "" {
						return fmt.Errorf("%s.%s: the member is the tag of the union %s, which the variant does not declare", v.Name, f.Name, u.Name)
					}
				}
			}
		}
	}
	return p.checkInlineMembers()
}

// checkInlineMembers refuses a struct whose inline union writes a member, its
// tag or its content, that another field of the struct has as its name.
func (p *reader) checkInlineMembers() error {
	for _, s := range p.pkg.Structs {
		for _, inline := range s.Fields {
			if !inline.Inline {
				continue
			}
			union := p.pkg.Unions[inline.Type.Name]
			for _, f := range s.Fields {
				if !f.Inline && (f.JSON == union.TagName || f.JSON == union.ContentName) {
					return fmt.Errorf("%s.%s: the member is one that the inline union %s writes", s.Name, f.Name, union.Name)
				}
			}
		}
	}
	return nil
}
