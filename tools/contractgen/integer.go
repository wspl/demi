package main

import "go/types"

// integerDecoder selects the numbered-input rule without changing output types.
func (g *generator) integerDecoder(t types.Type, marks map[string]string, msg bool) string {
	if !has(marks, "integer") {
		if msg {
			return g.msgDecoder(t)
		}
		return g.decoder(t)
	}
	if p, ok := t.(*types.Pointer); ok {
		return "func(b []byte)(" + g.typeName(t) + ",error){return contract.Pointer(b," + g.integerDecoder(p.Elem(), marks, msg) + ")}"
	}
	helper := "Integer"
	if msg {
		helper = "MsgpackInteger"
	}
	// Parse the underlying type so a named type's codec cannot recurse or refuse
	// the alternate input spelling. Enclosing validation checks its named rules.
	return "func(b []byte)(" + g.typeName(t) + ",error){v,err:=contract." + helper + "[" + g.typeName(t.Underlying()) + "](b);return " + g.typeName(t) + "(v),err}"
}
