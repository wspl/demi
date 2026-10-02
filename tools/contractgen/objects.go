package main

import (
	"fmt"
	"go/types"
	"reflect"
	"strings"
)

// object flattens embedded value structs into their wire properties. Synthetic
// field names retain the Go selector used to read and assign the original field.
func (g *generator) object(d *definition) (*types.Struct, bool) {
	st, ok := d.typ.Underlying().(*types.Struct)
	if !ok {
		return nil, false
	}
	var fields []*types.Var
	var tags []string
	for i := 0; i < st.NumFields(); i++ {
		field := st.Field(i)
		if !field.Embedded() {
			fields = append(fields, field)
			tags = append(tags, st.Tag(i))
			continue
		}
		named, ok := field.Type().(*types.Named)
		if !ok || reflect.StructTag(st.Tag(i)).Get("json") != "" {
			g.err = fmt.Errorf("%s: %s: only untagged embedded value structs can be flattened", d.position, d.name)
			continue
		}
		child := g.defs[typeKey(named)]
		if child == nil || has(child.marks, "union") || has(child.marks, "variant") {
			g.err = fmt.Errorf("%s: %s: flatten requires a plain contract object", d.position, d.name)
			continue
		}
		nested, ok := g.object(child)
		if !ok {
			g.err = fmt.Errorf("%s: %s: flatten requires a plain contract object", d.position, d.name)
			continue
		}
		for j := 0; j < nested.NumFields(); j++ {
			f := nested.Field(j)
			selector := field.Name() + "." + f.Name()
			fields = append(fields, types.NewVar(f.Pos(), f.Pkg(), selector, f.Type()))
			tags = append(tags, nested.Tag(j))
			d.fields[selector] = child.fields[f.Name()]
		}
	}
	return types.NewStruct(fields, tags), true
}

// emitJSONFields serializes object properties explicitly, avoiding promotion of
// an embedded contract's MarshalJSON method over its parent's sibling fields.
func (g *generator) emitJSONFields(d *definition, st *types.Struct, tag, variant string) {
	g.line("func(v %s)MarshalJSON()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};fields:=[]contract.Field{}", d.name)
	if tag != "" {
		g.line("fields=append(fields,contract.Field{Name:%s,Value:%s})", q(tag), q(variant))
	}
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
		if len(opts) > 1 {
			if isPointer(f.Type()) {
				g.line("if v.%s!=nil{", f.Name())
			} else {
				g.line("if v.%s{", f.Name())
			}
		}
		g.line("fields=append(fields,contract.Field{Name:%s,Value:v.%s})", q(opts[0]), f.Name())
		if len(opts) > 1 {
			g.line("}")
		}
	}
	g.line("return contract.EncodeObject(fields)}")
}
