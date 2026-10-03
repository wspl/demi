package main

import (
	"go/types"
	"reflect"
	"strings"
)

// tupleUnion identifies the external tag representation inherited by a variant.
func (g *generator) tupleUnion(d *definition) bool {
	union, _, _ := strings.Cut(d.marks["variant"], " ")
	return union != "" && g.defs[union].marks["msgpack"] == "tuple"
}

// emitTupleVariant reads and writes a kept-record variant in declaration order.
func (g *generator) emitTupleVariant(d *definition, st *types.Struct) {
	_, tag, _ := strings.Cut(d.marks["variant"], " ")
	g.line(
		"func %sMsgpack(data []byte)(%s,error){return contract.DecodeMsgpack[%s](data)}",
		goName("Decode", d.name),
		d.name,
		d.name,
	)
	g.line(
		"func(v *%s) UnmarshalMsgpack(data []byte)error{fields,"+
			"err:=contract.MsgpackTuple(data,%s,%d);if err!=nil{return err};var "+
			"next %s",
		d.name,
		quote(tag),
		st.NumFields(),
		d.name,
	)
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		m := d.fields[f.Name()]
		key := reflect.StructTag(st.Tag(i)).Get("json")
		g.line("{raw:=fields[%d]", i)
		if has(m, "nullable") {
			g.line("if !contract.MsgpackNull(raw){")
		}
		g.line(
			"value,err:=%s(raw);if err!=nil{return contract.At(%s,err)};next.%s=value",
			g.msgFieldDecoder(f.Type(), m),
			quote(key),
			f.Name(),
		)
		if has(m, "nullable") {
			g.line("}")
		}
		g.line("}")
	}
	g.line("if err:=next.Validate();err!=nil{return err};*v=next;return nil}")
	g.line(
		"func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};fields:=[]any{}",
		d.name,
	)
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		value := g.msgValue(f, d.fields[f.Name()])
		g.line("fields=append(fields,%s)", value)
	}
	g.line("return contract.EncodeMsgpackTuple(%s,fields)}", quote(tag))
}

// msgValue applies the wire adapter for a timestamp field when encoding it.
func (g *generator) msgValue(f *types.Var, m map[string]string) string {
	value := "v." + f.Name()
	if has(m, "timestamp") {
		g.line("var timestamp%s any", f.Name())
		depth := 0
		typ := f.Type()
		for {
			pointer, ok := typ.(*types.Pointer)
			if !ok {
				break
			}
			g.line("if %s!=nil{", value)
			value = "(*" + value + ")"
			typ = pointer.Elem()
			depth++
		}
		g.line("timestamp%s=contract.MsgpackTimestampValue(%s)", f.Name(), value)
		for range depth {
			g.line("}")
		}
		value = "timestamp" + f.Name()
	}
	return value
}
