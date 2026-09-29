package wiregen

import (
	"fmt"
	"go/ast"
	"strconv"
)

// embedForeign retains the owner's flattened schema while delegating its codec.
func (p *reader) embedForeign(s *Struct, field *ast.Field, selector *ast.SelectorExpr) error {
	qualifier, ok := selector.X.(*ast.Ident)
	if !ok || p.foreign == nil || !p.use(qualifier.Name) {
		return p.errorf(field.Pos(), "%s: embeds a wire struct that is not a variant; cannot resolve foreign embedding", s.Name)
	}
	owner, err := p.foreign(p.imports[p.file][qualifier.Name])
	if err != nil {
		return err
	}
	embedded := owner.Structs[selector.Sel.Name]
	if field.Tag != nil || embedded == nil || embedded.variant || embedded.Scalar != nil || embedded.Unknown != nil || hasInline(embedded) {
		return p.errorf(field.Pos(), "%s: foreign embedding requires an untagged marked struct without retained members or an inline union", s.Name)
	}
	typ := &Type{Kind: KindStruct, Name: embedded.Name, Src: p.source(selector), Qualifier: qualifier.Name, Opaque: true}
	typ.Validator = qualifier.Name + ".Validate"
	if owner.Exported[embedded.Name] {
		typ.Validator += embedded.Name
	}
	s.Embeds = append(s.Embeds, embedded.Name)
	s.ForeignEmbeds = append(s.ForeignEmbeds, typ)
	for _, original := range embedded.Fields {
		f := *original
		f.Owner = typ
		f.Type, err = p.qualifyType(original.Type, owner, qualifier.Name)
		if err != nil {
			return err
		}
		s.Fields = append(s.Fields, &f)
	}
	return nil
}

// qualifyType makes an owner's schema types usable in the importing declaration.
func (p *reader) qualifyType(original *Type, owner *Package, qualifier string) (*Type, error) {
	if original == nil {
		return nil, nil
	}
	t := *original
	var err error
	t.Elem, err = p.qualifyType(original.Elem, owner, qualifier)
	if err != nil {
		return nil, err
	}
	t.Key, err = p.qualifyType(original.Key, owner, qualifier)
	if err != nil {
		return nil, err
	}
	if t.Name != "" {
		alias := qualifier
		if t.Qualifier != "" {
			path := t.ImportPath
			for _, imports := range owner.sourceImports {
				if path == "" && imports[t.Qualifier] != "" {
					path = imports[t.Qualifier]
					break
				}
			}
			if path == "" {
				return nil, fmt.Errorf("cannot resolve owner import %s", t.Qualifier)
			}
			alias = t.Qualifier
			if existing := p.imports[p.file][alias]; existing != "" && existing != path {
				alias = "wireowner" + strconv.Itoa(len(p.used[p.file]))
			}
			p.imports[p.file][alias] = path
			p.use(alias)
		}
		t.Src = alias + "." + t.Name
		t.Qualifier = alias
		t.ImportPath = p.imports[p.file][alias]
		t.Opaque = true
		t.Rules = nil
	} else {
		switch t.Kind {
		case KindPointer:
			t.Src = "*" + t.Elem.Src
		case KindSlice:
			t.Src = "[]" + t.Elem.Src
		case KindMap:
			t.Src = "map[" + t.Key.Src + "]" + t.Elem.Src
		}
	}
	return &t, nil
}
