package main

import (
	"fmt"
	"go/token"
	"go/types"
	"math/big"
	"strings"
)

// checkRuleType rejects markers that would be silently ignored on this shape.
func checkRuleType(t types.Type, m map[string]string, field bool) error {
	if has(m, "flatten") && !field {
		return fmt.Errorf("flatten is a field marker")
	}
	if has(m, "table") {
		return fmt.Errorf("table requires a package-level slice variable")
	}
	if m["msgpack"] == "tuple" && !has(m, "union") {
		return fmt.Errorf("msgpack tuple requires a union")
	}
	original := t
	for {
		p, ok := t.(*types.Pointer)
		if !ok {
			break
		}
		t = p.Elem()
	}
	if isJSON(t) && len(m) > 0 {
		return fmt.Errorf("arbitrary JSON does not support field rules")
	}
	formats := 0
	for _, key := range []string{"timestamp", "base64", "enum", "format"} {
		if has(m, key) {
			formats++
		}
	}
	if formats > 1 {
		return fmt.Errorf("timestamp, base64, enum and format cannot be combined")
	}
	basic, isBasic := t.Underlying().(*types.Basic)
	stringType := isBasic && basic.Info()&types.IsString != 0
	numeric := isBasic && basic.Info()&(types.IsInteger|types.IsFloat) != 0
	for _, key := range []string{"pattern", "enum", "timestamp", "base64", "id", "format"} {
		if key == "timestamp" && !field && integerTimestamp(t) {
			continue
		}
		if key == "base64" {
			if slice, ok := t.Underlying().(*types.Slice); ok && types.Identical(slice.Elem(), types.Typ[types.Uint8]) {
				continue
			}
		}
		if has(m, key) && !stringType {
			return fmt.Errorf("%s requires a string", key)
		}
	}
	if has(m, "length") {
		_, slice := t.Underlying().(*types.Slice)
		if !stringType && !slice {
			return fmt.Errorf("length requires a string or array")
		}
		for _, v := range bounds(m["length"]) {
			n, ok := new(big.Int).SetString(v, 10)
			if !ok || n.Sign() < 0 || !n.IsInt64() {
				return fmt.Errorf("length bounds must be nonnegative integers")
			}
		}
	}
	if has(m, "range") && !numeric {
		return fmt.Errorf("range requires a number")
	}
	for _, rule := range []string{"length", "range"} {
		b := bounds(m[rule])
		minimum, minOK := new(big.Rat).SetString(b["min"])
		maximum, maxOK := new(big.Rat).SetString(b["max"])
		if minOK && maxOK && minimum.Cmp(maximum) > 0 {
			return fmt.Errorf("%s minimum exceeds maximum", rule)
		}
		if rule == "range" && isBasic && basic.Info()&types.IsInteger != 0 {
			for _, v := range b {
				n, ok := new(big.Int).SetString(v, 10)
				if !ok {
					return fmt.Errorf("integer bounds must be integers")
				}
				bits := 64
				switch basic.Kind() {
				case types.Int8, types.Uint8:
					bits = 8
				case types.Int16, types.Uint16:
					bits = 16
				case types.Int32, types.Uint32:
					bits = 32
				}
				unsigned := basic.Info()&types.IsUnsigned != 0
				if unsigned {
					if n.Sign() < 0 || n.BitLen() > bits {
						return fmt.Errorf("bound exceeds integer representation")
					}
				} else {
					limit := new(big.Int).Lsh(big.NewInt(1), uint(bits-1))
					lower := new(big.Int).Neg(limit)
					upper := new(big.Int).Sub(limit, big.NewInt(1))
					if n.Cmp(lower) < 0 || n.Cmp(upper) > 0 {
						return fmt.Errorf("bound exceeds integer representation")
					}
				}
			}
		}
	}
	if has(m, "nullable") {
		_, iface := original.Underlying().(*types.Interface)
		if !field || !isPointer(original) && !iface {
			return fmt.Errorf("nullable requires a pointer or union field")
		}
	}
	if has(m, "enum") {
		seen := map[string]bool{}
		values := strings.Fields(m["enum"])
		if len(values) == 0 {
			return fmt.Errorf("enum requires values")
		}
		for _, v := range values {
			if seen[v] {
				return fmt.Errorf("duplicate enum value")
			}
			seen[v] = true
		}
	}
	for _, key := range []string{"strict", "tolerant"} {
		if has(m, key) {
			if _, ok := t.Underlying().(*types.Struct); !ok {
				return fmt.Errorf("%s requires an object", key)
			}
		}
	}
	if field {
		for _, key := range []string{"union", "variant", "strict", "tolerant", "id", "root", "msgpack", "check", "format", "schema"} {
			if has(m, key) {
				return fmt.Errorf("%s is a type marker", key)
			}
		}
	}
	if check := m["check"]; check != "" {
		if !token.IsIdentifier(check) {
			return fmt.Errorf("check requires a function name")
		}
	}
	return nil
}

// checkCustom verifies the generated call's signature without executing it.
func checkCustom(d *definition) error {
	name := d.marks["check"]
	if name == "" {
		return nil
	}
	obj := d.typ.Obj().Pkg().Scope().Lookup(name)
	if obj == nil {
		return fmt.Errorf("check function %s is absent", name)
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok || signature.Params().Len() != 1 || signature.Results().Len() != 1 || !types.Identical(signature.Params().At(0).Type(), d.typ) || !types.Identical(signature.Results().At(0).Type(), types.Universe.Lookup("error").Type()) {
		return fmt.Errorf("check function must have signature func(%s) error", d.name)
	}
	return nil
}

// checkPattern ports the Rust emitter's shared-regexp subset, also excluding
// Go-only escapes that ECMAScript's Unicode mode cannot parse.
func checkPattern(pattern string) error {
	chars := []rune(pattern)
	inClass := false
	for i := 0; i < len(chars); i++ {
		c := chars[i]
		next := rune(0)
		if i+1 < len(chars) {
			next = chars[i+1]
		}
		refused := false
		switch c {
		case '\\':
			if strings.ContainsRune("dDwWsSbBpPAzZQE123456789", next) || next == 'x' && i+2 < len(chars) && chars[i+2] == '{' {
				refused = true
			}
			i++
		case '[':
			if inClass {
				refused = true
			}
			first := next
			if next == '^' && i+2 < len(chars) {
				first = chars[i+2]
			}
			if first == ']' {
				refused = true
			}
			inClass = true
		case ']':
			inClass = false
		case '&', '-', '~':
			refused = inClass && next == c
		case '.':
			refused = !inClass
		case '(':
			refused = !inClass && next == '?' && (i+2 >= len(chars) || chars[i+2] != ':')
		}
		if refused {
			return fmt.Errorf("pattern is outside the shared Go/browser subset")
		}
	}
	return nil
}

func integerTimestamp(t types.Type) bool {
	b, ok := t.Underlying().(*types.Basic)
	return ok && b.Kind() == types.Int64
}
