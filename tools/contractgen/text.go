package main

// normalizeText emits the explicitly declared input transformations before bounds.
func (g *generator) normalizeText(d *definition, value, onError string) {
	switch d.marks["format"] {
	case "trimmed":
		g.line("%s=contract.Trim(%s)", value, value)
	case "email", "http-url":
		helper := "Email"
		if d.marks["format"] == "http-url" {
			helper = "HTTPURL"
		}
		g.line("{normalized,err:=contract.%s(%s);if err!=nil{%s};%s=normalized}", helper, value, onError, value)
	}
}
