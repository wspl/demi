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
	// enum and value mark a type named after a basic type.
	directiveEnum  = "//demi:enum"
	directiveValue = "//demi:value"
	// describe, check and schema add to a declaration: describe is a line of
	// the description its JSON Schema carries, check the rules of a value type,
	// and schema asks for the JSON Schema of a struct or a union.
	directiveDescribe = "//demi:describe"
	directiveCheck    = "//demi:check"
	directiveSchema   = "//demi:schema"
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
	return loadSource(sources, foreignLoader(dir))
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
	return loadSource(sources, nil)
}

func loadSource(sources map[string][]byte, foreign func(string) (*Package, error)) (*Package, error) {
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
		foreign: foreign,
		fset:    fset,
		files:   files,
		methods: map[string]map[string]bool{},
		pkg: &Package{
			MessagePack:   map[string]bool{},
			Exported:      map[string]bool{},
			OpaqueScalars: map[string]*Type{},
			Structs:       map[string]*Struct{},
			Unions:        map[string]*Union{},
			Opaque:        map[string]bool{},
			named:         map[string]*Type{},
			patterns:      map[string]string{},
		},
		specs:     map[string]*ast.StructType{},
		imports:   map[string]map[string]string{},
		used:      map[string]map[string]string{},
		declFile:  map[string]string{},
		enums:     map[string][]string{},
		resolved:  map[string]bool{},
		resolving: map[string]bool{},
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
	foreign func(string) (*Package, error)
	fset    *token.FileSet
	files   []*ast.File
	pkg     *Package
	// methods are the names of the methods of each type.
	methods map[string]map[string]bool
	// specs are the struct declarations of the marked structs.
	specs map[string]*ast.StructType
	// decls are the declarations to resolve once every name is known, and
	// values those of the types named after a basic type, which come first: a
	// field takes the rules of its type.
	decls  []marked
	values []marked
	// enums are the values of the typed string constants, by the type's name, in
	// the order of their declarations.
	enums map[string][]string
	// resolved are the structs whose fields are read, and resolving those
	// being read, which finds a struct that embeds itself.
	resolved  map[string]bool
	resolving map[string]bool
	// imports are the packages each file imports, by the name the file gives
	// them, and used those of them that its wire types name; declFile is the
	// file of each declaration, and file the one being resolved.
	imports  map[string]map[string]string
	used     map[string]map[string]string
	declFile map[string]string
	file     string
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

// kind returns the directive that says what the declaration is, and the
// arguments after it; the other directives only add to it.
func (m marked) kind() (string, []string, error) {
	var kind string
	var arguments []string
	for _, directive := range m.directives {
		fields := strings.Fields(directive)
		switch fields[0] {
		case directiveWire, directiveUnion, directiveVariant, directiveOpaque, directiveEnum, directiveValue:
			if kind != "" {
				return "", nil, fmt.Errorf("%s: a declaration has one directive", m.name)
			}
			kind, arguments = fields[0], fields[1:]
		case directiveDescribe, directiveCheck, directiveSchema, "//demi:msgpack", "//demi:export":
		default:
			return "", nil, fmt.Errorf("%s: unknown directive %s", m.name, fields[0])
		}
	}
	return kind, arguments, nil
}

// lines returns the text after each directive named name, one for each line.
func (m marked) lines(name string) []string {
	var lines []string
	for _, directive := range m.directives {
		if directive == name || strings.HasPrefix(directive, name+" ") {
			lines = append(lines, strings.TrimPrefix(strings.TrimPrefix(directive, name), " "))
		}
	}
	return lines
}

// has reports whether the declaration has the directive name.
func (m marked) has(name string) bool {
	return len(m.lines(name)) > 0
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
				p.enumsOf(decl)
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
	for _, m := range p.values {
		if err := p.resolveValue(m); err != nil {
			return err
		}
	}
	for _, m := range p.decls {
		if err := p.resolve(m.name); err != nil {
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
	kind, arguments, err := m.kind()
	if err != nil {
		return p.errorf(m.spec.Pos(), "%v", err)
	}
	p.declFile[m.name] = m.file
	if m.has("//demi:export") {
		if kind != directiveWire && kind != directiveUnion && kind != directiveEnum && kind != directiveValue {
			return p.errorf(m.spec.Pos(), "%s: export marks a struct, union, enum or value", m.name)
		}
		if lines := m.lines("//demi:export"); len(lines) != 1 || lines[0] != "" {
			return p.errorf(m.spec.Pos(), "%s: export takes no arguments", m.name)
		}
		p.pkg.Exported[m.name] = true
	}
	if m.has("//demi:msgpack") {
		if kind != directiveWire && kind != directiveUnion && kind != directiveEnum {
			return p.errorf(m.spec.Pos(), "%s: //demi:msgpack marks a wire struct, union or enum", m.name)
		}
		if lines := m.lines("//demi:msgpack"); len(lines) != 1 || lines[0] != "" {
			return p.errorf(m.spec.Pos(), "%s: //demi:msgpack takes no arguments", m.name)
		}
		p.pkg.MessagePack[m.name] = true
	}
	description := strings.Join(m.lines(directiveDescribe), "\n")
	if kind == "" {
		if m.has(directiveCheck) || m.has(directiveDescribe) || m.has(directiveSchema) {
			return p.errorf(m.spec.Pos(), "%s: a declaration with %s, %s or %s is a wire declaration", m.name, directiveCheck, directiveDescribe, directiveSchema)
		}
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
	if len(arguments) > 0 && !(kind == directiveWire && len(arguments) == 1 && arguments[0] == "open") && kind != directiveUnion && kind != directiveVariant && kind != directiveOpaque {
		return p.errorf(m.spec.Pos(), "%s: %s takes no argument here", m.name, kind)
	}
	if kind == directiveEnum && m.has(directiveCheck) {
		return p.errorf(m.spec.Pos(), "%s: an enum has no %s: its rule is its constants", m.name, directiveCheck)
	}
	if kind != directiveValue && kind != directiveEnum && m.has(directiveCheck) {
		return p.errorf(m.spec.Pos(), "%s: %s belongs to a %s type", m.name, directiveCheck, directiveValue)
	}
	if (kind != directiveWire && kind != directiveUnion) && m.has(directiveSchema) {
		return p.errorf(m.spec.Pos(), "%s: %s marks a struct or a union", m.name, directiveSchema)
	}
	if (kind == directiveOpaque) && m.has(directiveDescribe) {
		return p.errorf(m.spec.Pos(), "%s: an opaque type is described by its package", m.name)
	}
	switch kind {
	case directiveOpaque:
		if _, ok := m.spec.Type.(*ast.StructType); !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks a struct", m.name, kind)
		}
		if len(arguments) > 0 {
			if arguments[0] != "string" || len(arguments) > 2 {
				return p.errorf(m.spec.Pos(), "%s: opaque scalar is string with an optional format=NAME", m.name)
			}
			typ := &Type{Kind: KindStruct, Name: m.name, Src: m.name, Opaque: true, WireKind: KindString}
			if len(arguments) == 2 {
				format, ok := strings.CutPrefix(arguments[1], "format=")
				if !ok || format == "" {
					return p.errorf(m.spec.Pos(), "%s: opaque scalar format is format=NAME", m.name)
				}
				typ.Format = format
			}
			p.pkg.OpaqueScalars[m.name] = typ
		}
		// An opaque type has no generated code, so it has no file to write.
		p.pkg.Opaque[m.name] = true
		return nil
	case directiveEnum, directiveValue:
		ident, ok := m.spec.Type.(*ast.Ident)
		var basic *Type
		if ok && !m.spec.Assign.IsValid() {
			basic = basicType(ident.Name)
		}
		if basic == nil || basic.Kind != KindString {
			return p.errorf(m.spec.Pos(), "%s: %s marks a type named after string", m.name, kind)
		}
		named := *basic
		named.Name = m.name
		named.Src = m.name
		named.Description = description
		p.pkg.named[m.name] = &named
		// A value type has no generated code, so it has no file to write.
		p.values = append(p.values, *m)
		return nil
	case directiveWire:
		if _, ok := m.spec.Type.(*ast.StructType); !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks a struct", m.name, kind)
		}
		p.pkg.Structs[m.name] = &Struct{
			Name:        m.name,
			Doc:         m.doc,
			Description: description,
			Schema:      m.has(directiveSchema),
			Open:        len(arguments) == 1,
		}
		p.specs[m.name] = m.spec.Type.(*ast.StructType)
	case directiveVariant:
		s := &Struct{Name: m.name, Doc: m.doc, Description: description, variant: true}
		switch typ := m.spec.Type.(type) {
		case *ast.StructType:
			if len(arguments) > 2 || len(arguments) == 2 && arguments[1] != "open" && arguments[1] != "opaque" {
				return p.errorf(m.spec.Pos(), "%s: %s takes at most a tag, and `open` or `opaque` after it", m.name, kind)
			}
			if len(arguments) > 0 {
				s.Tag = arguments[0]
			}
			s.Open = len(arguments) == 2 && arguments[1] == "open"
			s.Opaque = len(arguments) == 2 && arguments[1] == "opaque"
			p.specs[m.name] = typ
		case *ast.Ident:
			basic := basicType(typ.Name)
			if basic == nil || m.spec.Assign.IsValid() || len(arguments) > 0 {
				return p.errorf(m.spec.Pos(), "%s: %s marks a struct, or a type named after a basic type", m.name, kind)
			}
			scalar := *basic
			scalar.Name = m.name
			scalar.Src = m.name
			s.Scalar = &scalar
		default:
			return p.errorf(m.spec.Pos(), "%s: %s marks a struct, or a type named after a basic type", m.name, kind)
		}
		p.pkg.Structs[m.name] = s
	case directiveUnion:
		iface, ok := m.spec.Type.(*ast.InterfaceType)
		if !ok {
			return p.errorf(m.spec.Pos(), "%s: %s marks an interface", m.name, kind)
		}
		u := &Union{Name: m.name, Doc: m.doc, Description: description, Schema: m.has(directiveSchema)}
		switch {
		case len(arguments) == 1 && arguments[0] == "untagged":
		case len(arguments) >= 1 && len(arguments) <= 2 && strings.HasPrefix(arguments[0], "tag="):
			u.TagName = strings.TrimPrefix(arguments[0], "tag=")
			if len(arguments) == 2 {
				if !strings.HasPrefix(arguments[1], "content=") {
					return p.errorf(m.spec.Pos(), "%s: a union is `tag=NAME` or `untagged`, and `tag=NAME` may be followed by `content=NAME`", m.name)
				}
				u.ContentName = strings.TrimPrefix(arguments[1], "content=")
			}
			if u.TagName == "" || len(arguments) == 2 && (u.ContentName == "" || u.ContentName == u.TagName) {
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
	case "uint8", "byte":
		return &Type{Kind: KindUint, Src: name, Bits: 8}
	case "uint16":
		return &Type{Kind: KindUint, Src: name, Bits: 16}
	case "uint32":
		return &Type{Kind: KindUint, Src: name, Bits: 32}
	case "uint64":
		return &Type{Kind: KindUint, Src: name, Bits: 64}
	case "float64":
		return &Type{Kind: KindFloat, Src: name, Bits: 64}
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

// enumsOf records the values of the constants of a type named after string,
// which are the members of a closed set: `const A T = "a"`.
func (p *reader) enumsOf(decl *ast.GenDecl) {
	if decl.Tok != token.CONST {
		return
	}
	for _, spec := range decl.Specs {
		spec := spec.(*ast.ValueSpec)
		typ, ok := spec.Type.(*ast.Ident)
		if !ok || len(spec.Names) != len(spec.Values) {
			continue
		}
		for _, value := range spec.Values {
			if text, ok := p.stringConstant(value); ok {
				p.enums[typ.Name] = append(p.enums[typ.Name], text)
			}
		}
	}
}

// stringConstant evaluates a string literal, or the name of a string constant.
func (p *reader) stringConstant(expr ast.Expr) (string, bool) {
	switch expr := expr.(type) {
	case *ast.BasicLit:
		if expr.Kind != token.STRING {
			return "", false
		}
		text, err := strconv.Unquote(expr.Value)
		return text, err == nil
	case *ast.Ident:
		value, ok := p.pkg.consts[expr.Name]
		if !ok || value.Kind() != constant.String {
			return "", false
		}
		return constant.StringVal(value), true
	}
	return "", false
}

// resolveValue reads the rules of a type named after string: the members of a
// closed set, or the rules of a value.
func (p *reader) resolveValue(m marked) error {
	named := p.pkg.named[m.name]
	kind, _, _ := m.kind()
	checks := m.lines(directiveCheck)
	if kind == directiveEnum {
		values := p.enums[m.name]
		if len(values) == 0 {
			return p.errorf(m.spec.Pos(), "%s: an enum has constants of its type: const A %s = \"a\"", m.name, m.name)
		}
		seen := map[string]bool{}
		for _, value := range values {
			if seen[value] {
				return p.errorf(m.spec.Pos(), "%s: two constants have the value %q", m.name, value)
			}
			seen[value] = true
		}
		named.Rules = []Rule{{Kind: RuleOneOf, Values: values}}
		return nil
	}
	if len(checks) == 0 {
		return p.errorf(m.spec.Pos(), "%s: a value has the rules of its type: %s RULES", m.name, directiveCheck)
	}
	rules, err := p.parseRules(strings.Join(checks, ","))
	if err != nil {
		return p.errorf(m.spec.Pos(), "%s: %v", m.name, err)
	}
	if err := p.checkRules(named, rules); err != nil {
		return p.errorf(m.spec.Pos(), "%s: %v", m.name, err)
	}
	named.Rules = rules
	return nil
}

// resolve reads the fields of a marked struct. The members of a struct it
// embeds are its own, as serde's flatten makes them, so they are read first.
func (p *reader) resolve(name string) error {
	s, ok := p.pkg.Structs[name]
	if !ok || s.Scalar != nil || s.Opaque || p.resolved[name] {
		return nil
	}
	if p.resolving[name] {
		return fmt.Errorf("%s: a wire struct embeds itself", name)
	}
	p.resolving[name] = true
	defer delete(p.resolving, name)
	outer := p.file
	p.file = p.declFile[name]
	defer func() { p.file = outer }()
	spec := p.specs[name]
	s.Check = p.methods[name]["check"]
	s.Normalize = p.methods[name]["normalizeWire"]
	for _, field := range spec.Fields.List {
		if len(field.Names) == 0 {
			if err := p.embed(s, field); err != nil {
				return err
			}
			continue
		}
		if len(field.Names) != 1 {
			return p.errorf(field.Pos(), "%s: a wire struct declares one field per line", name)
		}
		f, err := p.field(name, field)
		if err != nil {
			return err
		}
		if f.Inline && f.Type.Kind == KindMembers {
			if s.Unknown != nil || s.Open {
				return p.errorf(field.Pos(), "%s: one retained-member collection replaces open", name)
			}
			s.Unknown = f
			continue
		}
		if f.Inline {
			switch {
			case s.variant:
				return p.errorf(field.Pos(), "%s: a variant has no inline field", name)
			case slices.ContainsFunc(s.Fields, func(other *Field) bool { return other.Inline }):
				return p.errorf(field.Pos(), "%s: a struct has one inline field", name)
			}
		}
		s.Fields = append(s.Fields, f)
	}
	if s.Unknown != nil && slices.ContainsFunc(s.Fields, func(f *Field) bool { return f.Inline }) {
		return fmt.Errorf("%s: unknown members and an inline union cannot share a struct", name)
	}
	names := map[string]bool{}
	for _, f := range s.Fields {
		if names[f.JSON] {
			return fmt.Errorf("%s: two members are named %q", name, f.JSON)
		}
		names[f.JSON] = true
	}
	p.resolved[name] = true
	return nil
}

// embed adds the members of the wire struct that field embeds to s.
func (p *reader) embed(s *Struct, field *ast.Field) error {
	ident, _ := field.Type.(*ast.Ident)
	var embedded *Struct
	if ident != nil {
		embedded = p.pkg.Structs[ident.Name]
	}
	switch {
	case embedded == nil || embedded.Scalar != nil || embedded.variant:
		return p.errorf(field.Pos(), "%s: a wire struct embeds a wire struct that is not a variant", s.Name)
	case field.Tag != nil:
		return p.errorf(field.Pos(), "%s: an embedded struct has no tag: its members are the struct's own", s.Name)
	}
	if err := p.resolve(embedded.Name); err != nil {
		return err
	}
	if embedded.Unknown != nil {
		return p.errorf(field.Pos(), "%s: an embedded struct cannot retain unknown members", s.Name)
	}
	if slices.ContainsFunc(embedded.Fields, func(f *Field) bool { return f.Inline }) {
		return p.errorf(field.Pos(), "%s: %s has an inline union, which an embedding struct does not have", s.Name, embedded.Name)
	}
	if embedded.Check {
		s.EmbedChecks = append(s.EmbedChecks, embedded.Name)
	}
	for _, check := range embedded.EmbedChecks {
		s.EmbedChecks = append(s.EmbedChecks, embedded.Name+"."+check)
	}
	s.Embeds = append(s.Embeds, embedded.Name)
	s.Fields = append(s.Fields, embedded.Fields...)
	return nil
}

// field reads a field of the wire struct structName.
func (p *reader) field(structName string, field *ast.Field) (*Field, error) {
	name := field.Names[0].Name
	where := structName + "." + name
	if !ast.IsExported(name) {
		return nil, p.errorf(field.Pos(), "%s: a wire field is exported", where)
	}
	if field.Tag == nil {
		return nil, p.errorf(field.Pos(), "%s: a wire field has a json tag", where)
	}
	tagText, err := strconv.Unquote(field.Tag.Value)
	if err != nil {
		return nil, p.errorf(field.Pos(), "%s: %v", where, err)
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
			return nil, p.errorf(field.Pos(), "%s: the json option %q is not one of a wire field's", where, option)
		}
	}
	if jsonName == "-" || jsonName == "" && !inline {
		return nil, p.errorf(field.Pos(), "%s: a wire field has a json name", where)
	}
	if inline && jsonName != "" {
		return nil, p.errorf(field.Pos(), "%s: an inline field has no json name", where)
	}
	t, err := p.typeOf(field.Type)
	if err != nil {
		return nil, p.errorf(field.Pos(), "%s: %v", where, err)
	}
	if inline && t.Kind == KindMembers {
		if omitzero || tag.Get("check") != "" || tag.Get("msgpack") != "" {
			return nil, p.errorf(field.Pos(), "%s: a retained-member collection has only json:\",inline\"", where)
		}
		return &Field{Name: name, Type: t, Inline: true}, nil
	}
	if t.Kind == KindMembers && !inline {
		return nil, p.errorf(field.Pos(), "%s: wire.Members requires json:\",inline\"", where)
	}
	if inline {
		switch {
		case t.Kind != KindUnion || p.pkg.Unions[t.Name] == nil || p.pkg.Unions[t.Name].ContentName == "":
			return nil, p.errorf(field.Pos(), "%s: an inline field is an adjacently tagged union", where)
		case omitzero:
			return nil, p.errorf(field.Pos(), "%s: an inline field is required", where)
		}
	}
	var rules []Rule
	nullAsAbsent := false
	if check, ok := tag.Lookup("check"); ok {
		parts, err := splitTop(check)
		if err != nil {
			return nil, p.errorf(field.Pos(), "%s: %v", where, err)
		}
		kept := parts[:0]
		for _, part := range parts {
			if strings.TrimSpace(part) == "nullabsent" {
				if nullAsAbsent {
					return nil, p.errorf(field.Pos(), "%s: repeated nullabsent", where)
				}
				nullAsAbsent = true
			} else {
				kept = append(kept, part)
			}
		}
		check = strings.Join(kept, ",")
		if len(kept) > 0 {
			rules, err = p.parseRules(check)
		}
		if err != nil {
			return nil, p.errorf(field.Pos(), "%s: %v", where, err)
		}
	}
	nullable := slices.ContainsFunc(rules, func(rule Rule) bool { return rule.Kind == RuleNullable })
	rules = slices.DeleteFunc(rules, func(rule Rule) bool { return rule.Kind == RuleNullable })
	triState := nullable && omitzero && t.Kind == KindPointer && t.Elem.Kind == KindPointer
	switch {
	case nullAsAbsent && (nullable || t.Kind != KindPointer || !omitzero || t.Elem.Kind == KindPointer):
		return nil, p.errorf(field.Pos(), "%s: nullabsent requires an optional single pointer", where)
	case t.Kind == KindPointer && t.Elem.Kind == KindPointer && !triState:
		return nil, p.errorf(field.Pos(), "%s: a double pointer requires omitzero and nullable", where)
	case nullable && !triState && (t.Kind != KindPointer || omitzero):
		return nil, p.errorf(field.Pos(), "%s: a nullable field is a required pointer, without omitzero", where)
	case omitzero && t.Kind != KindPointer && t.Kind != KindBool:
		return nil, p.errorf(field.Pos(), "%s: an optional field is a pointer with omitzero", where)
	case t.Kind == KindPointer && !omitzero && !nullable:
		return nil, p.errorf(field.Pos(), "%s: a pointer field is optional and has omitzero", where)
	}
	rules = expandTypeRules(t, rules)
	if err := p.checkRules(t, rules); err != nil {
		return nil, p.errorf(field.Pos(), "%s: %v", where, err)
	}
	if err := p.foreignRules(t, rules); err != nil {
		return nil, p.errorf(field.Pos(), "%s: %v", where, err)
	}
	return &Field{
		Name:         name,
		JSON:         jsonName,
		Doc:          strings.TrimSpace(field.Doc.Text()),
		Type:         t,
		Required:     !omitzero,
		Nullable:     nullable,
		NullAsAbsent: nullAsAbsent,
		TriState:     triState,
		Encoding:     tag.Get("msgpack"),
		Inline:       inline,
		Rules:        rules,
	}, nil
}

// expandTypeRules returns the rules of a field of type t whose own rules are
// rules: the rules of t come first, and those of the types of its elements
// apply to its elements.
func expandTypeRules(t *Type, rules []Rule) []Rule {
	switch t.Kind {
	case KindPointer:
		return expandTypeRules(t.Elem, rules)
	case KindSlice:
		return withInner(rules, RuleEach, expandTypeRules(t.Elem, innerRules(rules, RuleEach)))
	case KindMap:
		rules = withInner(rules, RuleEach, expandTypeRules(t.Elem, innerRules(rules, RuleEach)))
		return withInner(rules, RuleKeys, expandTypeRules(t.Key, innerRules(rules, RuleKeys)))
	}
	combined := slices.Clone(t.Rules)
	for _, rule := range rules {
		if !slices.ContainsFunc(combined, func(existing Rule) bool { return reflect.DeepEqual(existing, rule) }) {
			combined = append(combined, rule)
		}
	}
	return combined
}

// innerRules returns the rules inside the rule of kind in rules, if it has one.
func innerRules(rules []Rule, kind RuleKind) []Rule {
	for _, rule := range rules {
		if rule.Kind == kind {
			return rule.Inner
		}
	}
	return nil
}

// withInner returns rules with inner as the rules inside the rule of kind,
// which it adds when there is none and inner is not empty.
func withInner(rules []Rule, kind RuleKind, inner []Rule) []Rule {
	rules = slices.Clone(rules)
	for i, rule := range rules {
		if rule.Kind == kind {
			rules[i].Inner = inner
			return rules
		}
	}
	if len(inner) == 0 {
		return rules
	}
	return append(rules, Rule{Kind: kind, Inner: inner})
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
		if scalar := p.pkg.OpaqueScalars[expr.Name]; scalar != nil {
			return scalar, nil
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
		if elem.Kind == KindPointer && elem.Elem.Kind == KindPointer {
			return nil, fmt.Errorf("more than two pointer levels")
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
		if elem.Kind == KindPointer {
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
		return &Type{Kind: KindMap, Key: key, Elem: elem, Src: src}, nil
	case *ast.SelectorExpr:
		x, ok := expr.X.(*ast.Ident)
		if ok && x.Name == "jsontext" && expr.Sel.Name == "Value" {
			return &Type{Kind: KindRaw, Src: src}, nil
		}
		if ok && expr.Sel.Name == "Members" && p.imports[p.file][x.Name] == wireImport {
			// Generated retained-member code uses the runtime's own import.
			return &Type{Kind: KindMembers, Src: src}, nil
		}
		// A wire struct of another package: its own generated code decodes it,
		// so it is opaque here.
		if ok && p.use(x.Name) {
			if p.foreign != nil {
				owner, err := p.foreign(p.imports[p.file][x.Name])
				if err != nil {
					return nil, err
				}
				if opaque := owner.OpaqueScalars[expr.Sel.Name]; opaque != nil {
					typ := *opaque
					typ.Src = src
					typ.Qualifier = x.Name
					return &typ, nil
				}
				if owner.Unions[expr.Sel.Name] != nil {
					return &Type{Kind: KindUnion, Name: expr.Sel.Name, Src: src, Opaque: true, Qualifier: x.Name}, nil
				}
				if named := owner.named[expr.Sel.Name]; named != nil {
					typ := *named
					typ.Name = expr.Sel.Name
					typ.Src = src
					typ.Qualifier = x.Name
					typ.Opaque = true
					typ.Rules = nil
					return &typ, nil
				}
			}
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
		isVariant := s.variant
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
		if s.Scalar != nil && owner.TagName != "" {
			return p.errorf(m.spec.Pos(), "%s: a variant that is not an object belongs to an untagged union", m.name)
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
		if err := checkKinds(u); err != nil {
			return err
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

// checkKinds refuses an untagged union with two variants of one JSON kind that
// are not objects: nothing could tell them apart.
func checkKinds(u *Union) error {
	kinds := map[string]string{}
	for _, v := range u.Variants {
		if v.Scalar == nil {
			continue
		}
		kind := v.Scalar.jsonKind()
		if other, ok := kinds[kind]; ok {
			return fmt.Errorf("%s: the variants %s and %s of the union are both a JSON %s", v.Name, other, v.Name, kind)
		}
		kinds[kind] = v.Name
	}
	return nil
}
