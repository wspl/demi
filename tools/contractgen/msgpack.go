package main

import (
	"go/types"
	"reflect"
	"strings"
)

// msgDecoder selects the generated decoder for a runner field's Go shape.
func (g *generator) msgDecoder(t types.Type) string {
	if isJSON(t) {
		return "contract.MsgpackJSON"
	}
	switch t := t.(type) {
	case *types.Pointer:
		return "func(b []byte)(" + g.typeName(t) + ",error){ return contract.Pointer(b," + g.msgDecoder(t.Elem()) + ") }"
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return "contract.DecodeMsgpack[[]byte]"
		}
		return "func(b []byte)(" + g.typeName(t) + ",error){return contract.MsgpackList(b," + g.msgDecoder(t.Elem()) + ")}"
	case *types.Map:
		nullable := "false"
		if isPointer(t.Elem()) {
			nullable = "true"
		}
		return "func(b []byte)(" + g.typeName(t) + ",error){return contract.MsgpackRecord(b," + g.msgDecoder(t.Elem()) + "," + nullable + ")}"
	case *types.Named:
		if _, ok := t.Underlying().(*types.Interface); ok {
			return g.prefix(t) + "Decode" + t.Obj().Name() + "Msgpack"
		}
	}
	return "contract.DecodeMsgpack[" + g.typeName(t) + "]"
}

// emitMsgpack derives the runner codec from the same declarations as JSON.
func (g *generator) emitMsgpack(d *definition) {
	name := d.name
	if has(d.marks, "union") {
		tag := bounds(d.marks["union"])["tag"]
		if d.marks["msgpack"] == "tuple" {
			g.line("func Decode%sMsgpack(data []byte)(%s,error){obj,err:=contract.MsgpackObject(data);if err!=nil{return nil,err};if len(obj)!=1{return nil,fmt.Errorf(\"expected one external union tag\")};var tag string;for key:=range obj{tag=key};switch tag{", name, name)
		} else if d.marks["union"] == "untagged" {
			g.line("func Decode%sMsgpack(data []byte)(%s,error){", name, name)
			for _, v := range g.variants(d.key) {
				g.line("if value,err:=contract.DecodeMsgpack[%s](data);err==nil{return &value,nil}", v.name)
			}
			g.line("return nil,fmt.Errorf(\"no matching union variant\")} ")
			g.line("func Encode%sMsgpack(v %s)([]byte,error){if err:=Validate%s(v);err!=nil{return nil,err};return contract.EncodeMsgpack(v)}", name, name, name)
			return
		} else {
			g.line("func Decode%sMsgpack(data []byte)(%s,error){ obj,err:=contract.MsgpackObject(data);if err!=nil{return nil,err};tag,err:=contract.DecodeMsgpack[%s](obj[%s]);if err!=nil{return nil,contract.At(%s,err)};switch tag{", name, name, g.unionKind(d), q(tag), q(tag))
		}
		for _, v := range g.variants(d.key) {
			_, value, _ := strings.Cut(v.marks["variant"], " ")
			g.line("case %s: value,err:=contract.DecodeMsgpack[%s](data);return &value,err", tagLiteral(value), v.name)
		}
		g.line("};return nil,fmt.Errorf(\"unknown union tag\")}")
		g.line("func Encode%sMsgpack(v %s)([]byte,error){if err:=Validate%s(v);err!=nil{return nil,err};return contract.EncodeMsgpack(v)}", name, name, name)
		return
	}
	if g.tupleUnion(d) {
		st, _ := g.object(d)
		g.emitTupleVariant(d, st)
		return
	}
	g.line("func Decode%sMsgpack(data []byte)(%s,error){return contract.DecodeMsgpack[%s](data)}", name, name, name)
	if g.adjacentUnion(d) != nil {
		st, _ := g.object(d)
		g.emitAdjacentVariant(d, st, true)
		return
	}
	g.line("func (v *%s) UnmarshalMsgpack(data []byte)error{", name)
	if has(d.marks, "timestamp") && integerTimestamp(d.typ) {
		g.line("value,err:=contract.MsgpackMillis(data);if err!=nil{return err};next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;return nil}", name)
		g.line("func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};return contract.EncodeMsgpackMillis(int64(v))}", name)
		return
	}
	if has(d.marks, "timestamp") {
		g.line("value,err:=contract.MsgpackTimestamp(data);if err!=nil{return err};next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;return nil}", name)
		g.line("func(v %s) MarshalMsgpack()([]byte,error){return contract.EncodeMsgpackTimestamp(string(v))}", name)
		return
	}
	st, ok := g.object(d)
	if !ok {
		g.line("value,err:=%s(data);if err!=nil{return err}", g.msgDecoder(d.typ.Underlying()))
		g.normalizeText(d, "value", "return err")
		g.line("next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;return nil}", name)
		g.line("func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};return contract.EncodeMsgpack(%s(v))}", name, g.typeName(d.typ.Underlying()))
		return
	}
	tag, variant, kind := g.variantWire(d)
	g.line("obj,err:=contract.MsgpackObject(data);if err!=nil{return err};var next %s", name)
	if tag != "" {
		g.line("tag,err:=contract.DecodeMsgpack[%s](obj[%s]);if err!=nil{return contract.At(%s,err)};if tag!=%s{return fmt.Errorf(\"invalid union tag\")};delete(obj,%s)", kind, q(tag), q(tag), tagLiteral(variant), q(tag))
	}
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		if g.flattenedUnion(d, f) != nil {
			g.emitFlattenDecode(d, f, true)
			continue
		}
		opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
		key := opts[0]
		m := d.fields[f.Name()]
		if len(opts) > 1 && emptyCollection(f.Type()) {
			g.line("next.%s=make(%s,0)", f.Name(), g.typeName(f.Type()))
		}
		g.line("{raw,present:=obj[%s];delete(obj,%s)", q(key), q(key))
		if len(opts) == 1 {
			g.line("if !present{return contract.At(%s,fmt.Errorf(\"required field is absent\"))}", q(key))
		}
		g.line("if present{")
		if has(m, "nullable") {
			g.line("if !contract.MsgpackNull(raw){")
		}
		g.line("value,err:=%s(raw);if err!=nil{return contract.At(%s,err)};next.%s=value", g.msgFieldDecoder(f.Type(), m), q(key), f.Name())
		if has(m, "nullable") {
			g.line("}")
		}
		g.line("}}")
	}
	if !has(d.marks, "tolerant") {
		g.line("for key:=range obj{return contract.At(key,fmt.Errorf(\"unknown field\"))}")
	}
	g.line("if err:=next.Validate();err!=nil{return err};*v=next;return nil}")
	g.line("func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();err!=nil{return nil,err};fields:=[]contract.Field{}", name)
	if tag != "" {
		g.line("fields=append(fields,contract.Field{Name:%s,Value:%s})", q(tag), tagLiteral(variant))
	}
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		if g.flattenedUnion(d, f) != nil {
			g.emitFlattenEncode(f, true)
			continue
		}
		opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
		if len(opts) > 1 {
			if isPointer(f.Type()) {
				g.line("if v.%s!=nil{", f.Name())
			} else if emptyCollection(f.Type()) {
				g.line("if len(v.%s)>0{", f.Name())
			} else {
				g.line("if v.%s{", f.Name())
			}
		}
		value := g.msgValue(f, d.fields[f.Name()])
		g.line("fields=append(fields,contract.Field{Name:%s,Value:%s})", q(opts[0]), value)
		if len(opts) > 1 {
			g.line("}")
		}
	}
	g.line("return contract.EncodeMsgpackObject(fields)}")
}

// msgFieldDecoder applies scalar wire annotations before the parent validator.
func (g *generator) msgFieldDecoder(t types.Type, m map[string]string) string {
	if !has(m, "timestamp") {
		return g.msgDecoder(t)
	}
	if p, ok := t.(*types.Pointer); ok {
		return "func(b []byte)(" + g.typeName(t) + ",error){return contract.Pointer(b," + g.msgFieldDecoder(p.Elem(), m) + ")}"
	}
	return "func(b []byte)(" + g.typeName(t) + ",error){v,err:=contract.MsgpackTimestamp(b);return " + g.typeName(t) + "(v),err}"
}
