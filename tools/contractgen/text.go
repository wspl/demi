package main

// normalizeText emits the explicitly declared input transformations before bounds.
func (g *generator) normalizeText(d *definition, value, onError string) {
	switch d.marks["format"] {
	case "trimmed":
		g.line("%s=contract.Trim(%s)", value, value)
	case "email":
		g.line("{normalized,err:=contract.Email(%s);if err!=nil{%s};%s=normalized}", value, onError, value)
	}
}
