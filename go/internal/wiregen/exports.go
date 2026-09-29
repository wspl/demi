package wiregen

import (
	"maps"
	"slices"
	"strings"
)

// exports gives foreign consumers and enum boundaries their owner's entry points
// without changing the existing generated codecs or requiring MessagePack.
func (p *Package) exports() ([]byte, error) {
	if len(p.Exported) == 0 {
		return nil, nil
	}
	g := &goGen{imports: map[string]bool{wireImport: true, "encoding/json/v2": true}}
	packed := p.messagePackTypes()
	for _, name := range slices.Sorted(maps.Keys(p.Exported)) {
		union := p.Unions[name]
		named := p.named[name]
		if named != nil && !packed[name] && !named.CustomCheck {
			g.p("func(v %s) validate() error {", name)
			g.p("var r wire.Report")
			g.value(named, "v", named.Rules, `""`, "r", 0)
			g.p("return r.Err()")
			g.p("}")
		}
		if union != nil || named != nil {
			g.imports["encoding/json/jsontext"] = true
			if !packed[name] {
				g.p("// Decode%sJSONFrom reads a %s with its owner's codec.", name, name)
				g.p("func Decode%sJSONFrom(dec *jsontext.Decoder) (%s,error) {", name, name)
				if union != nil {
					g.p("return decode%s(dec)", name)
				} else {
					if named.CustomDecode {
						g.p("var value %s", name)
						g.p("if err:=value.UnmarshalJSONFrom(dec);err!=nil{return value,err}")
					} else {
						g.p("text,err:=wire.ReadString(dec)")
						g.p("if err!=nil{return \"\",err}")
						g.p("value:=%s(text)", name)
					}
					if named.CustomCheck {
						g.p("return value,Validate%s(value)", name)
					} else {
						g.p("return value,value.validate()")
					}
				}
				g.p("}")
			}
		}
		g.p("// Validate%s checks a %s using its owner's rules.", name, name)
		g.p("func Validate%s(value %s) error {", name, name)
		if union != nil {
			g.p("return validate%s(value)", name)
		} else if named != nil && named.CustomCheck {
			g.p("var r wire.Report")
			g.value(named, "value", named.Rules, `""`, "r", 0)
			g.p("return r.Err()")
		} else {
			g.p("return value.validate()")
		}
		g.p("}")
		g.p("// Decode%sJSON reads and validates one complete JSON value.", name)
		g.p("func Decode%sJSON(data []byte) (%s,error) {", name, name)
		g.p("var value %s", name)
		if union != nil || named != nil {
			g.p("err:=json.Unmarshal(data,&value,json.WithUnmarshalers(json.UnmarshalFromFunc(func(dec *jsontext.Decoder,v *%s) error {", name)
			g.p("decoded,err:=Decode%sJSONFrom(dec)", name)
			g.p("if err==nil {*v=decoded}")
			g.p("return err")
			g.p("})))")
		} else {
			g.p("err:=json.Unmarshal(data,&value)")
		}
		g.p("if err!=nil{return value,wire.Refusal(err)}")
		g.p("return value,Validate%s(value)", name)
		g.p("}")
		g.p("// Encode%sJSON validates and encodes a %s.", name, name)
		g.p("func Encode%sJSON(value %s) ([]byte,error) {", name, name)
		g.p("if err:=Validate%s(value);err!=nil{return nil,err}", name)
		g.p("data,err:=json.Marshal(value,json.Deterministic(true))")
		g.p("if err!=nil{return nil,wire.Refusal(err)}")
		g.p("return data,nil")
		g.p("}")
	}
	for _, file := range p.Files {
		for alias, path := range file.Imports {
			if strings.Contains(g.buf.String(), alias+".") {
				g.imports[path] = true
				if g.aliases == nil {
					g.aliases = map[string]string{}
				}
				g.aliases[path] = alias
			}
		}
	}
	return g.finish(p.Name)
}
