package main

import "go/types"

// optionalObject identifies a flattened optional contract object, retaining its
// boundary so decoding can try the entire child as serde's Option does.
func (g *generator) optionalObject(f *types.Var) *definition {
	if !f.Embedded() {
		return nil
	}
	p, ok := f.Type().(*types.Pointer)
	if !ok {
		return nil
	}
	n, ok := p.Elem().(*types.Named)
	if !ok {
		return nil
	}
	d := g.defs[typeKey(n)]
	if d == nil || has(d.marks, "codec") || has(d.marks, "union") || has(d.marks, "variant") {
		return nil
	}
	if _, ok := n.Underlying().(*types.Struct); !ok {
		return nil
	}
	return d
}

// emitOptionalObjectDecode tries the child's contributed properties together.
// Serde's flattened Option deliberately turns a failed child decode into None.
func (g *generator) emitOptionalObjectDecode(f *types.Var, child *definition, msg bool) {
	st, _ := g.object(child)
	fields, encode, decoder := "ObjectFields", "EncodeObject", g.decoder(child.typ)
	if msg {
		fields, encode, decoder = "MsgpackFields", "EncodeMsgpackObject", g.msgDecoder(child.typ)
	}
	g.line(
		"{parts:=[]contract.Field{};source,err:=contract.%s(data);if "+
			"err!=nil{return err};for _,field:=range source{switch field.Name{",
		fields,
	)
	for _, key := range g.propertyNames(child, st) {
		g.line("case %s:parts=append(parts,field)", quote(key))
		if msg {
			g.line("delete(obj,field.Name)")
		}
	}
	g.line(
		"}};raw,err:=contract.%s(parts);if err!=nil{return err};if value,err:=%s(raw);err==nil{next.%s=&value}}",
		encode,
		decoder,
		f.Name(),
	)
}
