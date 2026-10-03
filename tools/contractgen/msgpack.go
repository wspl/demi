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
		return "func(b []byte)(" + g.typeName(
			t,
		) + ",error){ return contract.Pointer(b," + g.msgDecoder(
			t.Elem(),
		) + ") }"
	case *types.Slice:
		if types.Identical(t.Elem(), types.Typ[types.Uint8]) {
			return "contract.DecodeMsgpack[[]byte]"
		}
		return "func(b []byte)(" + g.typeName(
			t,
		) + ",error){return contract.MsgpackList(b," + g.msgDecoder(
			t.Elem(),
		) + ")}"
	case *types.Map:
		nullable := "false"
		if isPointer(t.Elem()) {
			nullable = "true"
		}
		if !types.Identical(t.Key(), types.Typ[types.String]) {
			return "func(b []byte)(" + g.typeName(
				t,
			) + ",error){return contract.MsgpackKeyedRecord[" + g.typeName(
				t.Key(),
			) + "](b," + g.msgDecoder(
				t.Elem(),
			) + "," + nullable + ")}"
		}
		return "func(b []byte)(" + g.typeName(
			t,
		) + ",error){return contract.MsgpackRecord(b," + g.msgDecoder(
			t.Elem(),
		) + "," + nullable + ")}"
	case *types.Named:
		if _, ok := t.Underlying().(*types.Interface); ok {
			return g.prefix(t) + goName("Decode", t.Obj().Name()) + "Msgpack"
		}
	}
	return "contract.DecodeMsgpack[" + g.typeName(t) + "]"
}

// emitMsgpack derives the runner codec from the same declarations as JSON.
func (g *generator) emitMsgpack(d *definition) {
	if has(d.marks, "codec") {
		return
	}
	name := d.name
	if has(d.marks, "union") {
		g.emitMsgpackUnion(d)
		return
	}
	if g.tupleUnion(d) {
		st, _ := g.object(d)
		g.emitTupleVariant(d, st)
		return
	}
	g.line(
		"func %sMsgpack(data []byte)(%s,error){return contract.DecodeMsgpack[%s](data)}",
		goName("Decode", name),
		name,
		name,
	)
	if g.adjacentUnion(d) != nil {
		st, _ := g.object(d)
		g.emitAdjacentVariant(d, st, true)
		return
	}
	g.line("func (v *%s) UnmarshalMsgpack(data []byte)error{", name)
	if g.emitMsgpackTimestamp(d) {
		return
	}
	st, ok := g.object(d)
	if !ok {
		g.line("value,err:=%s(data);if err!=nil{return err}", g.integerDecoder(d.typ.Underlying(), d.marks, true))
		g.normalizeText(d, "value", "return err")
		g.line("next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;return nil}", name)
		g.line(
			"func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();"+
				"err!=nil{return nil,err};return contract.EncodeMsgpack(%s(v))}",
			name,
			g.typeName(d.typ.Underlying()),
		)
		return
	}
	g.emitMsgpackObject(d, st)
}

// msgFieldDecoder applies scalar wire annotations before the parent validator.
func (g *generator) msgFieldDecoder(t types.Type, m map[string]string) string {
	if !has(m, "timestamp") {
		return g.integerDecoder(t, m, true)
	}
	if p, ok := t.(*types.Pointer); ok {
		return "func(b []byte)(" + g.typeName(
			t,
		) + ",error){return contract.Pointer(b," + g.msgFieldDecoder(
			p.Elem(),
			m,
		) + ")}"
	}
	return "func(b []byte)(" + g.typeName(
		t,
	) + ",error){v,err:=contract.MsgpackTimestamp(b);return " + g.typeName(
		t,
	) + "(v),err}"
}

func (g *generator) emitMsgpackUnion(d *definition) {
	name := d.name
	tag := bounds(d.marks["union"])["tag"]
	if d.marks["msgpack"] == "tuple" {
		g.line(
			"func %sMsgpack(data []byte)(%s,error){obj,"+
				"err:=contract.MsgpackObject(data);if err!=nil{return nil,err};if "+
				"len(obj)!=1{return nil,fmt.Errorf(\"expected one external union tag\")};"+
				"var tag string;for key:=range obj{tag=key};switch tag{",
			goName("Decode", name),
			name,
		)
	} else if d.marks["union"] == "untagged" {
		g.line("func %sMsgpack(data []byte)(%s,error){", goName("Decode", name), name)
		for _, v := range g.variants(d.key) {
			g.line("if value,err:=contract.DecodeMsgpack[%s](data);err==nil{return &value,nil}", v.name)
		}
		g.line("return nil,fmt.Errorf(\"no matching union variant\")} ")
		g.line(
			"func %sMsgpack(v %s)([]byte,error){if err:=%s(v);err!=nil{return nil,err};return contract.EncodeMsgpack(v)}",
			goName("Encode", name),
			name,
			goName("Validate", name),
		)
		return
	} else {
		g.line(
			"func %sMsgpack(data []byte)(%s,error){ obj,"+
				"err:=contract.MsgpackObject(data);if err!=nil{return nil,err};tag,"+
				"err:=contract.DecodeMsgpack[%s](obj[%s]);if err!=nil{return nil,"+
				"contract.At(%s,err)};switch tag{",
			goName("Decode", name),
			name,
			g.unionKind(d),
			quote(tag),
			quote(tag),
		)
	}
	for _, v := range g.variants(d.key) {
		_, value, _ := strings.Cut(v.marks["variant"], " ")
		g.line("case %s: value,err:=contract.DecodeMsgpack[%s](data);return &value,err", tagLiteral(value), v.name)
	}
	g.line("};return nil,fmt.Errorf(\"unknown union tag\")}")
	g.line(
		"func %sMsgpack(v %s)([]byte,error){if err:=%s(v);err!=nil{return nil,err};return contract.EncodeMsgpack(v)}",
		goName("Encode", name),
		name,
		goName("Validate", name),
	)
}

func (g *generator) emitMsgpackObject(d *definition, st *types.Struct) {
	name := d.name
	tag, variant, kind := g.variantWire(d)
	object := "obj"
	if st.NumFields() == 0 && tag == "" && has(d.marks, "tolerant") {
		object = "_"
	}
	g.line("%s,err:=contract.MsgpackObject(data);if err!=nil{return err};var next %s", object, name)
	if tag != "" {
		g.line(
			"tag,err:=contract.DecodeMsgpack[%s](obj[%s]);if err!=nil{return "+
				"contract.At(%s,err)};if tag!=%s{return fmt.Errorf(\"invalid union "+
				"tag\")};delete(obj,%s)",
			kind,
			quote(tag),
			quote(tag),
			tagLiteral(variant),
			quote(tag),
		)
	}
	for i := 0; i < st.NumFields(); i++ {
		g.emitMsgpackFieldDecode(d, st, i)
	}
	if !has(d.marks, "tolerant") {
		g.line("for key:=range obj{return contract.At(key,fmt.Errorf(\"unknown field\"))}")
	}
	g.line("if err:=next.Validate();err!=nil{return err};*v=next;return nil}")
	g.line("func(v %s) MarshalMsgpack()([]byte,error){", name)
	g.emitDefaultCollections(d, st)
	g.line("if err:=v.Validate();err!=nil{return nil,err};fields:=[]contract.Field{}")
	if tag != "" {
		g.line("fields=append(fields,contract.Field{Name:%s,Value:%s})", quote(tag), tagLiteral(variant))
	}
	for i := 0; i < st.NumFields(); i++ {
		g.emitMsgpackFieldEncode(d, st, i)
	}
	g.line("return contract.EncodeMsgpackObject(fields)}")
}

func (g *generator) emitMsgpackFieldDecode(d *definition, st *types.Struct, i int) {
	f := st.Field(i)
	if child := g.optionalObject(f); child != nil {
		g.emitOptionalObjectDecode(f, child, true)
		return
	}
	if g.flattenedUnion(d, f) != nil {
		g.emitFlattenDecode(d, f, true)
		return
	}
	opts := strings.Split(reflect.StructTag(st.Tag(i)).Get("json"), ",")
	key := opts[0]
	m := d.fields[f.Name()]
	if (len(opts) > 1 || has(m, "default")) && emptyCollection(f.Type()) {
		g.line("next.%s=make(%s,0)", f.Name(), g.typeName(f.Type()))
	}
	g.line("{raw,present:=obj[%s];delete(obj,%s)", quote(key), quote(key))
	if len(opts) == 1 && !has(m, "default") {
		g.line("if !present{return contract.At(%s,fmt.Errorf(\"required field is absent\"))}", quote(key))
	}
	g.line("if present{")
	if has(m, "nullable") {
		if pointer, ok := f.Type().(*types.Pointer); len(opts) > 1 && ok && isPointer(pointer.Elem()) {
			g.line("if contract.MsgpackNull(raw){next.%s=new(%s)}else{", f.Name(), g.typeName(pointer.Elem()))
		} else {
			g.line("if !contract.MsgpackNull(raw){")
		}
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
	g.line("}}")
}

func (g *generator) emitMsgpackFieldEncode(d *definition, st *types.Struct, i int) {
	f := st.Field(i)
	if g.optionalObject(f) != nil {
		g.line("if v.%s!=nil{", f.Name())
		g.emitFlattenEncode(f, true)
		g.line("}")
		return
	}
	if g.flattenedUnion(d, f) != nil {
		g.emitFlattenEncode(f, true)
		return
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
	g.line("fields=append(fields,contract.Field{Name:%s,Value:%s})", quote(opts[0]), value)
	if len(opts) > 1 {
		g.line("}")
	}
}

// emitMsgpackTimestamp reports whether it emitted a complete timestamp codec.
func (g *generator) emitMsgpackTimestamp(d *definition) bool {
	name := d.name
	if has(d.marks, "timestamp") && integerTimestamp(d.typ) {
		g.line(
			"value,err:=contract.MsgpackMillis(data);if err!=nil{return err};"+
				"next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;"+
				"return nil}",
			name,
		)
		g.line(
			"func(v %s) MarshalMsgpack()([]byte,error){if err:=v.Validate();"+
				"err!=nil{return nil,err};return "+
				"contract.EncodeMsgpackMillis(int64(v))}",
			name,
		)
		return true
	}
	if has(d.marks, "timestamp") {
		g.line(
			"value,err:=contract.MsgpackTimestamp(data);if err!=nil{return err};"+
				"next:=%s(value);if err:=next.Validate();err!=nil{return err};*v=next;"+
				"return nil}",
			name,
		)
		g.line("func(v %s) MarshalMsgpack()([]byte,error){return contract.EncodeMsgpackTimestamp(string(v))}", name)
		return true
	}

	return false
}
