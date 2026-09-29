// Package schematest compares JSON Schemas: two schemas that mean the same are
// the same text once normalized, so a test can hold a generated schema to one
// that another implementation derived.
package schematest

import (
	"encoding/json/jsontext"
	"fmt"
	"slices"
	"strconv"
	"strings"
)

// Normalize writes a JSON document so that two documents with the same meaning
// are the same text: numbers by their value, and the members of an object by
// name, except the properties of a schema, whose order is what the model reads
// (help lists a command's parameters in it).
func Normalize(document []byte) (string, error) {
	dec := jsontext.NewDecoder(strings.NewReader(string(document)))
	var out strings.Builder
	if err := normalize(dec, &out, false); err != nil {
		return "", err
	}
	return out.String(), nil
}

func normalize(dec *jsontext.Decoder, out *strings.Builder, ordered bool) error {
	token, err := dec.ReadToken()
	if err != nil {
		return err
	}
	switch token.Kind() {
	case '{':
		type member struct{ name, value string }
		var members []member
		for dec.PeekKind() != '}' {
			token, err := dec.ReadToken()
			if err != nil {
				return err
			}
			name := token.String()
			var value strings.Builder
			if err := normalize(dec, &value, name == "properties"); err != nil {
				return err
			}
			members = append(members, member{name, value.String()})
		}
		if _, err := dec.ReadToken(); err != nil {
			return err
		}
		if !ordered {
			slices.SortFunc(members, func(a, b member) int { return strings.Compare(a.name, b.name) })
		}
		out.WriteByte('{')
		for i, m := range members {
			if i > 0 {
				out.WriteByte(',')
			}
			fmt.Fprintf(out, "%s:%s", strconv.Quote(m.name), m.value)
		}
		out.WriteByte('}')
	case '[':
		out.WriteByte('[')
		for i := 0; dec.PeekKind() != ']'; i++ {
			if i > 0 {
				out.WriteByte(',')
			}
			if err := normalize(dec, out, false); err != nil {
				return err
			}
		}
		if _, err := dec.ReadToken(); err != nil {
			return err
		}
		out.WriteByte(']')
	case '0':
		number, err := token.Float()
		if err != nil {
			return err
		}
		out.WriteString(strconv.FormatFloat(number, 'g', -1, 64))
	case '"':
		out.WriteString(strconv.Quote(token.String()))
	default:
		out.WriteString(token.String())
	}
	return nil
}
