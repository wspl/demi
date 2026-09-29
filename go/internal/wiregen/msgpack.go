package wiregen

import (
	"fmt"
	"maps"
	"slices"
	"strconv"
	"strings"
)

const msgpImport = "github.com/tinylib/msgp/msgp"

// messagePack writes additional codecs for explicitly selected roots and their
// local dependencies. Existing JSON outputs do not depend on that selection.
func (p *Package) messagePack() ([]byte, error) {
	selected := p.messagePackTypes()
	if len(selected) == 0 {
		return nil, nil
	}
	g := &mpGen{goGen: goGen{imports: map[string]bool{wireImport: true, msgpImport: true}, unions: p.Unions}}
	for _, file := range p.Files {
		for _, name := range file.Types {
			if !selected[name] {
				continue
			}
			if s := p.Structs[name]; s != nil && !s.Opaque {
				for _, f := range s.Fields {
					typ := f.Type
					for typ.Kind == KindPointer {
						typ = typ.Elem
					}
					if f.Encoding == "bin" && (typ.Kind != KindSlice || typ.Elem.Kind != KindUint || typ.Elem.Bits != 8) {
						return nil, fmt.Errorf("%s.%s: bin requires a byte slice", name, f.Name)
					}
					if f.Encoding == "timestamp" && (typ.Kind != KindInt || typ.Bits != 64) {
						return nil, fmt.Errorf("%s.%s: timestamp requires int64 milliseconds", name, f.Name)
					}
					if f.Encoding != "" && f.Encoding != "bin" && f.Encoding != "timestamp" {
						return nil, fmt.Errorf("%s.%s: unknown MessagePack encoding", name, f.Name)
					}
				}
				if err := g.structureMP(s); err != nil {
					return nil, err
				}
			}
			if u := p.Unions[name]; u != nil {
				g.unionMP(u)
			}
		}
		for alias, path := range file.Imports {
			if g.imports[path] || strings.Contains(g.buf.String(), alias+".") {
				g.imports[path] = true
				if g.aliases == nil {
					g.aliases = map[string]string{}
				}
				g.aliases[path] = alias
			}
		}
	}
	for _, name := range slices.Sorted(maps.Keys(p.named)) {
		if selected[name] && len(p.named[name].Rules) > 0 {
			g.enumMP(p.named[name])
		}
	}
	return g.finish(p.Name)
}

type mpGen struct{ goGen }

// decoder names a typed read callback. The generator, rather than reflection,
// selects every member's representation and every nested contract.
func (g *mpGen) decoder(t *Type, encoding string) string {
	body := ""
	switch {
	case t.Qualifier != "" && (t.Kind == KindUnion || t.Kind == KindString):
		body = fmt.Sprintf("return %s.Decode%sMsgpack(data)", t.Qualifier, t.Name)
	case t.Kind == KindPointer:
		body = fmt.Sprintf("return wire.MPNullable(data, %s)", g.decoder(t.Elem, encoding))
	case encoding == "bin":
		body = "return wire.MPBytes(data)"
	case encoding == "timestamp":
		body = fmt.Sprintf("n, err := wire.MPTimestamp(data); return %s(n), err", t.Src)
	case t.Kind == KindSlice:
		body = fmt.Sprintf("return wire.MPArray(data, %s)", g.decoder(t.Elem, ""))
	case t.Kind == KindMap:
		body = fmt.Sprintf("return wire.MPMap[%s](data, %s)", t.Key.Src, g.decoder(t.Elem, ""))
	case t.Kind == KindUnion:
		body = fmt.Sprintf("return decode%sMsgpack(data)", t.Name)
	case t.Kind == KindStruct:
		body = fmt.Sprintf("var v %s; err := v.UnmarshalMsgpack(data); return v, err", t.Src)
	case t.Kind == KindRaw:
		g.imports["encoding/json/jsontext"] = true
		body = "return wire.MPJSON(data)"
	default:
		read := ""
		switch t.Kind {
		case KindString:
			read = "wire.MPString(data)"
		case KindBool:
			read = "wire.MPRead(data, msgp.ReadBoolBytes)"
		case KindFloat:
			read = "wire.MPFloat(data)"
		case KindInt, KindUint:
			base := "Int"
			if t.Kind == KindUint {
				base = "Uint"
			}
			if t.Bits != 0 {
				base += strconv.Itoa(t.Bits)
			}
			read = "wire.MPRead(data, msgp.Read" + base + "Bytes)"
		}
		body = fmt.Sprintf("value, err := %s; return %s(value), err", read, t.Src)
	}
	return fmt.Sprintf("func(data []byte) (%s,error) { %s }", t.Src, strings.ReplaceAll(body, "; ", "\n"))
}

func (g *mpGen) structureMP(s *Struct) error {

	g.p("// UnmarshalMsgpack reads the structure of one complete %s value.", s.Name)
	g.p("func (v *%s) UnmarshalMsgpack(data []byte) error {", s.Name)
	if s.Scalar != nil {
		g.p("value,err := (%s)(data)", g.decoder(s.Scalar, ""))
		g.p("if err != nil {return err}")
		g.p("*v = value")
		g.p("return nil")
		g.p("}")
		g.p("func (v %s) MarshalMsgpack() ([]byte,error) {", s.Name)
		g.p("var data []byte")
		g.appendMP(s.Scalar, "v", "", `""`)
		g.p("return data,nil")
		g.p("}")
		return nil
	}
	g.p("*v = %s{}", s.Name)
	adjacent := s.Union != nil && s.Union.ContentName != ""
	if adjacent {
		g.p("content,err := wire.MPAdjacent(data,%q,%q,%q)", s.Union.TagName, s.Union.ContentName, s.Tag)
		g.p("if err != nil {return err}")
		g.p("return wire.NestIn(%q, v.unmarshalMsgpackContent(content))", s.Union.ContentName)
		g.p("}")
		g.p("func (v *%s) unmarshalMsgpackContent(data []byte) error {", s.Name)
	}
	g.p("fields, err := wire.MPObject(data)")
	g.p("if err != nil { return err }")
	if len(s.Fields) > 0 {
		g.p("seen := map[string]bool{}")
	}
	if hasInline(s) {
		g.p("var inlineFields []wire.MPMember")
	}
	tagged := s.Union != nil && s.Union.TagName != "" && s.Union.ContentName == ""
	if tagged {
		g.p("tagSeen := false")
	}
	g.p("for _,field := range fields {")
	g.p("switch field.Name {")
	if tagged {
		g.p("case %q:", s.Union.TagName)
		g.p("tagSeen = true")
		g.p("tag, err := wire.MPString(field.Data)")
		g.p("if err != nil { return wire.In(%q,err) }", s.Union.TagName)
		g.p("if tag != %q { return wire.UnknownTag(%q,%q) }", s.Tag, s.Union.TagName, s.Tag)
	}
	for _, f := range s.Fields {
		if f.Inline {
			u := g.unions[f.Type.Name]
			g.p("case %q,%q:", u.TagName, u.ContentName)
			g.p("inlineFields = append(inlineFields,field)")
			continue
		}
		g.p("case %q:", f.JSON)
		g.p("seen[field.Name] = true")
		t := f.Type
		if f.NullAsAbsent {
			g.p("if msgp.IsNil(field.Data) {continue}")
		}
		if t.Kind == KindPointer && (!f.Nullable || f.TriState) {
			g.p("value,err := (%s)(field.Data)", g.decoder(t.Elem, f.Encoding))
			g.p("if err != nil { return wire.In(field.Name,err) }")
			g.p("v.%s = &value", f.Name)
		} else {
			g.p("value,err := (%s)(field.Data)", g.decoder(t, f.Encoding))
			g.p("if err != nil { return wire.In(field.Name,err) }")
			g.p("v.%s = value", f.Name)
		}
	}
	g.p("default:")
	if s.Unknown != nil {
		g.p("raw,err:=wire.MPJSON(field.Data)")
		g.p("if err!=nil{return wire.In(field.Name,err)}")
		g.p("v.%s=append(v.%s,wire.Member{Name:field.Name,Value:raw})", s.Unknown.Name, s.Unknown.Name)
	} else if !s.Open {
		g.p("return wire.Unknown(field.Name)")
	}
	g.p("}")
	g.p("}")
	if tagged {
		g.p("if !tagSeen {return wire.Required(%q)}", s.Union.TagName)
	}
	for _, f := range s.Fields {
		if f.Required && !f.Inline {
			g.p("if !seen[%q] {return wire.Required(%q)}", f.JSON, f.JSON)
		}
	}
	for _, f := range s.Fields {
		if f.Inline {
			g.p("inline := msgp.AppendMapHeader(nil,uint32(len(inlineFields)))")
			g.p("for _,field := range inlineFields {inline = msgp.AppendString(inline,field.Name);inline=append(inline,field.Data...)}")
			g.p("v.%s,err = decode%sMsgpack(inline)", f.Name, f.Type.Name)
			g.p("if err != nil {return err}")
		}
	}
	g.p("return nil")
	g.p("}")
	g.p("// Decode%sMsgpack reads and validates one complete value.", s.Name)
	g.p("func Decode%sMsgpack(data []byte) (%s,error) {", s.Name, s.Name)
	g.p("var value %s", s.Name)
	g.p("if err := value.UnmarshalMsgpack(data); err != nil {return value,err}")
	g.p("return value,value.validate()")
	g.p("}")
	g.p("// MarshalMsgpack encodes %s as a map in declaration order.", s.Name)
	g.p("func (v %s) MarshalMsgpack() ([]byte,error) {", s.Name)
	if s.Normalize {
		g.p("var err error; v,err = v.normalizeWire(); if err != nil {return nil,err}")
	}
	g.p("if err := v.validate(); err != nil {return nil,err}")
	n := len(s.Fields)
	if hasInline(s) {
		n++
	}
	if tagged {
		n++
	}
	g.p("count := uint32(%d)", n)
	if s.Unknown != nil {
		g.p("count += uint32(len(v.%s))", s.Unknown.Name)
	}
	for _, f := range s.Fields {
		if !f.Required {
			if f.Type.Kind == KindBool {
				g.p("if !v.%s {count--}", f.Name)
			} else {
				g.p("if v.%s == nil {count--}", f.Name)
			}
		}
	}
	g.p("data := msgp.AppendMapHeader(nil,count)")
	if tagged {
		g.p("data = msgp.AppendString(data,%q)", s.Union.TagName)
		g.p("data = msgp.AppendString(data,%q)", s.Tag)
	}
	for _, f := range s.Fields {
		if f.Inline {
			g.p("{")
			g.p("encoded,err := Encode%sMsgpack(v.%s)", f.Type.Name, f.Name)
			g.p("if err != nil {return nil,err}")
			g.p("_,members,err := msgp.ReadMapHeaderBytes(encoded)")
			g.p("if err != nil {return nil,err}")
			g.p("data = append(data,members...)")
			g.p("}")
			continue
		}
		if !f.Required {
			if f.Type.Kind == KindBool {
				g.p("if v.%s {", f.Name)
			} else {
				g.p("if v.%s != nil {", f.Name)
			}
		}
		g.p("data = msgp.AppendString(data,%q)", f.JSON)
		g.p("{")
		g.appendMP(f.Type, "v."+f.Name, f.Encoding, strconv.Quote(f.JSON))
		g.p("}")
		if !f.Required {
			g.p("}")
		}
	}
	if s.Unknown != nil {
		g.p("for _,member:=range v.%s {", s.Unknown.Name)
		g.p("data=msgp.AppendString(data,member.Name)")
		g.p("raw,err:=wire.JSONMsgpack(member.Value)")
		g.p("if err!=nil{return nil,wire.In(member.Name,err)}")
		g.p("data=append(data,raw...)")
		g.p("}")
	}
	if adjacent {
		g.p("envelope := msgp.AppendMapHeader(nil,2)")
		g.p("envelope = msgp.AppendString(envelope,%q)", s.Union.TagName)
		g.p("envelope = msgp.AppendString(envelope,%q)", s.Tag)
		g.p("envelope = msgp.AppendString(envelope,%q)", s.Union.ContentName)
		g.p("return append(envelope,data...),nil")
	} else {
		g.p("return data,nil")
	}
	g.p("}")
	return nil
}

// appendMP emits a field's encoder using the runtime's MessagePack primitives.
func (g *mpGen) appendMP(t *Type, expr, encoding, path string) {
	switch {
	case t.Kind == KindPointer:
		g.p("if %s == nil { data = msgp.AppendNil(data) } else {", expr)
		g.appendMP(t.Elem, "(*"+expr+")", encoding, path)
		g.p("}")
	case encoding == "bin":
		g.p("data = msgp.AppendBytes(data,%s)", expr)
	case encoding == "timestamp":
		g.p("data = wire.MPAppendTimestamp(data,int64(%s))", expr)
	case t.Kind == KindString && t.Qualifier != "":
		g.p("encoded,err := %s.Encode%sMsgpack(%s)", t.Qualifier, t.Name, expr)
		g.p("if err != nil {return nil,wire.In(%s,err)}", path)
		g.p("data=append(data,encoded...)")
	case t.Kind == KindString:
		g.imports["unicode/utf8"] = true
		g.p("if !utf8.ValidString(string(%s)) {return nil, &wire.InvalidError{Path:%s,Rule:\"invalid UTF-8\"}}", expr, path)
		g.p("data = msgp.AppendString(data,string(%s))", expr)
	case t.Kind == KindBool:
		g.p("data = msgp.AppendBool(data,bool(%s))", expr)
	case t.Kind == KindInt:
		g.p("if %s >= 0 {data = msgp.AppendUint64(data,uint64(%s))} else {data = msgp.AppendInt64(data,int64(%s))}", expr, expr, expr)
	case t.Kind == KindUint:
		g.p("data = msgp.AppendUint64(data,uint64(%s))", expr)
	case t.Kind == KindFloat:
		g.imports["math"] = true
		g.p("if math.IsNaN(float64(%s)) || math.IsInf(float64(%s),0) {return nil,&wire.InvalidError{Path:%s,Rule:\"expected finite number\"}}", expr, expr, path)
		g.p("data = msgp.AppendFloat64(data,float64(%s))", expr)
	case t.Kind == KindSlice:
		g.p("data = msgp.AppendArrayHeader(data,uint32(len(%s)))", expr)
		g.p("for index, item := range %s {", expr)
		g.p("itemPath := wire.Prefix(%s,wire.Index(index))", path)
		g.p("_ = itemPath")
		g.appendMP(t.Elem, "item", "", "itemPath")
		g.p("}")
	case t.Kind == KindMap:
		g.imports["slices"] = true
		g.imports["maps"] = true
		g.p("data = msgp.AppendMapHeader(data,uint32(len(%s)))", expr)
		g.p("for _, key := range slices.Sorted(maps.Keys(%s)) {", expr)
		g.p("if !utf8.ValidString(string(key)) {return nil,&wire.InvalidError{Path:%s,Rule:\"invalid UTF-8 map key\"}}", path)
		g.imports["unicode/utf8"] = true
		g.p("data = msgp.AppendString(data,string(key))")
		g.p("item := %s[key]", expr)
		g.p("itemPath := wire.Prefix(%s,wire.Key(string(key)))", path)
		g.p("_ = itemPath")
		g.appendMP(t.Elem, "item", "", "itemPath")
		g.p("}")
	default:
		switch t.Kind {
		case KindUnion:
			prefix := ""
			if t.Qualifier != "" {
				prefix = t.Qualifier + "."
			}
			g.p("encoded,err := %sEncode%sMsgpack(%s)", prefix, t.Name, expr)
		case KindRaw:
			g.p("encoded,err := wire.JSONMsgpack(%s)", expr)
		default:
			g.p("encoded,err := %s.MarshalMsgpack()", expr)
		}
		g.p("if err != nil {return nil,wire.In(%s,err)}", path)
		g.p("data = append(data,encoded...)")
	}
}

func (g *mpGen) unionMP(u *Union) {
	g.imports["encoding/json/jsontext"] = true
	g.p("// Decode%sJSONFrom reads the union using its owner's JSON codec.", u.Name)
	g.p("func Decode%sJSONFrom(dec *jsontext.Decoder) (%s,error) {return decode%s(dec)}", u.Name, u.Name, u.Name)

	g.p("// Decode%sMsgpack decodes and checks one variant of %s.", u.Name, u.Name)
	g.p("func Decode%sMsgpack(data []byte) (%s,error) {", u.Name, u.Name)
	g.p("value,err := decode%sMsgpack(data)", u.Name)
	g.p("if err != nil {return nil,err}")
	g.p("return value,validate%s(value)", u.Name)
	g.p("}")
	g.p("func decode%sMsgpack(data []byte) (%s,error) {", u.Name, u.Name)
	if u.TagName != "" {
		g.p("fields,err := wire.MPObject(data)")
		g.p("if err != nil {return nil,err}")
		g.p("var tag string")
		g.p("found := false")
		g.p("for _,field := range fields { if field.Name == %q {", u.TagName)
		g.p("tag,err = wire.MPString(field.Data)")
		g.p("if err != nil {return nil,wire.In(%q,err)}", u.TagName)
		g.p("found = true")
		g.p("} }")
		g.p("if !found {return nil,wire.Required(%q)}", u.TagName)
		g.p("switch tag {")
		for _, v := range u.Variants {
			g.p("case %q:", v.Tag)
			g.p("var value %s", v.Name)
			g.p("if err := value.UnmarshalMsgpack(data); err != nil {return nil,err}")
			g.p("return value,nil")
		}
		g.p("}")
		tags := []string{}
		for _, v := range u.Variants {
			tags = append(tags, v.Tag)
		}
		g.p("return nil,wire.UnknownTag(%q,%s)", u.TagName, quoted(tags))
	} else {
		g.p("var found %s", u.Name)
		g.p("matches := 0")
		for _, v := range u.Variants {
			g.p("{var value %s; if value.UnmarshalMsgpack(data) == nil {found=value; matches++}}", v.Name)
		}
		g.p("if matches == 1 {return found,nil}")
		g.p("if matches == 0 {return nil,wire.NoVariant(%s)}", quoted(variantNames(u)))
		g.p("return nil,wire.SeveralVariants(%s)", quoted(variantNames(u)))
	}
	g.p("}")
	g.p("// Encode%sMsgpack checks and encodes the selected variant.", u.Name)
	g.p("func Encode%sMsgpack(value %s) ([]byte,error) {", u.Name, u.Name)
	g.p("if err := validate%s(value); err != nil {return nil,err}", u.Name)
	g.p("switch value := value.(type) {")
	for _, v := range u.Variants {
		for _, prefix := range []string{"", "*"} {
			g.p("case %s%s: return value.MarshalMsgpack()", prefix, v.Name)
		}
	}
	g.p("}")
	g.p("return nil,wire.NoVariant(%s)", quoted(variantNames(u)))
	g.p("}")
}

// enumMP gives foreign callers the owning package's closed-set decoder and check.
func (g *mpGen) enumMP(t *Type) {
	g.imports["encoding/json/jsontext"] = true
	g.p("func (v %s) validate() error {", t.Name)
	g.p("var r wire.Report")
	g.value(t, "v", t.Rules, `""`, "r", 0)
	g.p("return r.Err()")
	g.p("}")
	g.p("func Decode%sJSONFrom(dec *jsontext.Decoder) (%s,error) {", t.Name, t.Name)
	g.p("text,err := wire.ReadString(dec)")
	g.p("if err != nil {return \"\",err}")
	g.p("v := %s(text)", t.Name)
	g.p("return v,v.validate()")
	g.p("}")
	g.p("func Decode%sMsgpack(data []byte) (%s,error) {", t.Name, t.Name)
	g.p("text,err := wire.MPString(data)")
	g.p("if err != nil {return \"\",err}")
	g.p("v := %s(text)", t.Name)
	g.p("return v,v.validate()")
	g.p("}")
	g.p("func Encode%sMsgpack(v %s) ([]byte,error) {", t.Name, t.Name)
	g.p("if err:=v.validate();err!=nil{return nil,err}")
	g.p("return msgp.AppendString(nil,string(v)),nil")
	g.p("}")
}

// messagePackTypes follows the local dependencies of opted-in roots.
func (p *Package) messagePackTypes() map[string]bool {
	selected := map[string]bool{}
	var visit func(string)
	var field func(*Type)
	field = func(t *Type) {
		switch t.Kind {
		case KindStruct, KindUnion, KindString:
			if t.Qualifier == "" && !t.Opaque && t.Name != "" {
				visit(t.Name)
			}
		case KindPointer, KindSlice, KindMap:
			field(t.Elem)
		}
	}
	visit = func(name string) {
		if selected[name] {
			return
		}
		selected[name] = true
		if s := p.Structs[name]; s != nil {
			for _, f := range s.Fields {
				field(f.Type)
			}
		}
		if u := p.Unions[name]; u != nil {
			for _, s := range u.Variants {
				visit(s.Name)
			}
		}
	}
	for name := range p.MessagePack {
		visit(name)
	}
	return selected
}
