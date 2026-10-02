package main

import (
	"fmt"
	"go/types"
	"reflect"
	"strings"
)

// adjacentUnion finds the content-bearing wire representation of a variant.
func (g *generator) adjacentUnion(d *definition) *definition {
	if len(d.unions) == 0 {
		return nil
	}
	u := g.defs[d.unions[0]]
	if bounds(u.marks["union"])["content"] == "" {
		return nil
	}
	return u
}

// flattenedUnion resolves a field whose union contributes sibling properties.
func (g *generator) flattenedUnion(d *definition, f *types.Var) *definition {
	if !has(d.fields[f.Name()], "flatten") {
		return nil
	}
	n, ok := f.Type().(*types.Named)
	if !ok {
		return nil
	}
	return g.defs[typeKey(n)]
}

// emitAdjacentVariant writes the operation-selected scalar or nil payload codec.
func (g *generator) emitAdjacentVariant(d *definition, st *types.Struct, msg bool) {
	u := g.adjacentUnion(d)
	args := bounds(u.marks["union"])
	tag, content := args["tag"], args["content"]
	_, variant, kind := g.variantWire(d)
	suffix, fields, encode, decode, null := "JSON", "ObjectFields", "EncodeObject", "contract.Decode", "contract.IsNull"
	if msg {
		suffix, fields, encode, decode, null = "Msgpack", "MsgpackFields", "EncodeMsgpackObject", "contract.DecodeMsgpack", "contract.MsgpackNull"
	}
	g.line("func(v *%s) Unmarshal%s(data []byte)error{fields,err:=contract.%s(data);if err!=nil{return err};if _,err:=contract.AdjacentFields(fields,%s,%s);err!=nil{return err}", d.name, suffix, fields, q(tag), q(content))
	if !has(d.marks, "tolerant") {
		g.line("if len(fields)!=2{return fmt.Errorf(\"expected adjacent tag and content\")}")
	}
	if msg {
		g.line("obj,err:=contract.MsgpackObject(data)")
	} else {
		g.line("obj,err:=contract.Decode[map[string]json.RawMessage](data)")
	}
	g.line("if err!=nil{return err};tag,err:=%s[%s](obj[%s]);if err!=nil{return contract.At(%s,err)};if tag!=%s{return fmt.Errorf(\"invalid union tag\")};var next %s", decode, kind, q(tag), q(tag), tagLiteral(variant), d.name)
	if st.NumFields() == 0 {
		g.line("if !%s(obj[%s]){return contract.At(%s,fmt.Errorf(\"expected nil content\"))}", null, q(content), q(content))
	} else {
		f := st.Field(0)
		decoder := g.integerDecoder(f.Type(), d.fields[f.Name()], false)
		if msg {
			decoder = g.msgFieldDecoder(f.Type(), d.fields[f.Name()])
		}
		if has(d.fields[f.Name()], "nullable") {
			g.line("if !%s(obj[%s]){", null, q(content))
		}
		g.line("value,err:=%s(obj[%s]);if err!=nil{return contract.At(%s,err)};next.%s=value", decoder, q(content), q(content), f.Name())
		if has(d.fields[f.Name()], "nullable") {
			g.line("}")
		}
	}
	g.line("if err:=next.Validate();err!=nil{return err};*v=next;return nil}")
	g.line("func(v %s) Marshal%s()([]byte,error){if err:=v.Validate();err!=nil{return nil,err}", d.name, suffix)
	value := "nil"
	if st.NumFields() > 0 {
		value = "v." + st.Field(0).Name()
		if msg {
			value = g.msgValue(st.Field(0), d.fields[st.Field(0).Name()])
		}
	}
	g.line("return contract.%s([]contract.Field{{Name:%s,Value:%s},{Name:%s,Value:%s}})}", encode, q(tag), tagLiteral(variant), q(content), value)
}

// emitFlattenDecode extracts the union keys while preserving the parent's order.
func (g *generator) emitFlattenDecode(d *definition, f *types.Var, msg bool) {
	u := g.flattenedUnion(d, f)
	args := bounds(u.marks["union"])
	fields, encode, decoder := "ObjectFields", "EncodeObject", g.decoder(f.Type())
	if msg {
		fields, encode, decoder = "MsgpackFields", "EncodeMsgpackObject", g.msgDecoder(f.Type())
	}
	g.line("{fields,err:=contract.%s(data);if err!=nil{return err};parts,err:=contract.AdjacentFields(fields,%s,%s);if err!=nil{return err};raw,err:=contract.%s(parts);if err!=nil{return err};value,err:=%s(raw);if err!=nil{return err};next.%s=value}", fields, q(args["tag"]), q(args["content"]), encode, decoder, f.Name())
	if msg {
		g.line("delete(obj,%s);delete(obj,%s)", q(args["tag"]), q(args["content"]))
	}
}

// emitFlattenEncode inserts the selected operation and content at the field position.
func (g *generator) emitFlattenEncode(f *types.Var, msg bool) {
	encode, fields := "EncodeJSON", "ObjectFields"
	if msg {
		encode, fields = "EncodeMsgpack", "MsgpackFields"
	}
	g.line("{raw,err:=contract.%s(v.%s);if err!=nil{return nil,err};parts,err:=contract.%s(raw);if err!=nil{return nil,err};fields=append(fields,parts...)}", encode, f.Name(), fields)
}

// propertyNames gives the keys contributed by ordinary and flattened fields.
func (g *generator) propertyNames(d *definition, st *types.Struct) []string {
	keys := []string{}
	active := map[string]bool{}
	var visit func(*definition, *types.Struct)
	visit = func(d *definition, st *types.Struct) {
		if active[d.key] {
			g.err = fmt.Errorf("%s: %s: recursive flattened object is unsupported", d.position, d.name)
			return
		}
		active[d.key] = true
		defer delete(active, d.key)
		for i := 0; i < st.NumFields(); i++ {
			f := st.Field(i)
			if child := g.optionalObject(f); child != nil {
				nested, _ := g.object(child)
				visit(child, nested)
			} else if u := g.flattenedUnion(d, f); u != nil {
				args := bounds(u.marks["union"])
				keys = append(keys, args["tag"], args["content"])
			} else {
				keys = append(keys, strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")[0])
			}
		}
	}
	visit(d, st)
	return keys
}
