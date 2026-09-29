package rustfmt

import (
	"bytes"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"math"
	"strconv"
	"strings"
)

// maxJSONDepth is how deeply serde_json reads arrays and objects nested in one
// another: one level fewer than its recursion limit of 128.
const maxJSONDepth = 127

// NormalizeJSON returns the JSON document data as serde_json prints the Value it
// reads from it: members in the order they were written, a repeated key once, at
// its first place with its last value, strings escaped only where they must be,
// and each number as the integer or float it reads as: a number that is not an
// integer that fits 64 bits is the float it rounds to, printed by [JSONFloat]. It
// fails where serde_json does: for a document that is not JSON, a number out of
// range, a string with an unpaired surrogate escape, and nesting deeper than 127
// levels.
func NormalizeJSON(data []byte) ([]byte, error) {
	decoder := jsontext.NewDecoder(bytes.NewReader(data), jsontext.AllowDuplicateNames(true))
	var out []byte
	out, err := appendNormalized(out, decoder, 0)
	if err != nil {
		return nil, err
	}
	if _, err := decoder.ReadToken(); err == nil {
		return nil, errors.New("trailing characters")
	}
	return out, nil
}

// appendNormalized appends the next value of decoder, normalized.
func appendNormalized(out []byte, decoder *jsontext.Decoder, depth int) ([]byte, error) {
	token, err := decoder.ReadToken()
	if err != nil {
		return nil, err
	}
	switch token.Kind() {
	case 'n':
		return append(out, "null"...), nil
	case 't':
		return append(out, "true"...), nil
	case 'f':
		return append(out, "false"...), nil
	case '"':
		return AppendJSONString(out, token.String()), nil
	case '0':
		return appendNumber(out, token.String())
	case '[':
		if depth >= maxJSONDepth {
			return nil, errors.New("recursion limit exceeded")
		}
		out = append(out, '[')
		for first := true; decoder.PeekKind() != ']'; first = false {
			if !first {
				out = append(out, ',')
			}
			if out, err = appendNormalized(out, decoder, depth+1); err != nil {
				return nil, err
			}
		}
		if _, err := decoder.ReadToken(); err != nil {
			return nil, err
		}
		return append(out, ']'), nil
	case '{':
		if depth >= maxJSONDepth {
			return nil, errors.New("recursion limit exceeded")
		}
		return appendObject(out, decoder, depth)
	}
	return nil, fmt.Errorf("unexpected token %q", token.String())
}

// appendObject appends the members of the object whose opening brace was read.
func appendObject(out []byte, decoder *jsontext.Decoder, depth int) ([]byte, error) {
	var keys []string
	values := map[string][]byte{}
	for decoder.PeekKind() != '}' {
		token, err := decoder.ReadToken()
		if err != nil {
			return nil, err
		}
		// A token is valid until the next read.
		key := token.String()
		value, err := appendNormalized(nil, decoder, depth+1)
		if err != nil {
			return nil, err
		}
		if _, repeated := values[key]; !repeated {
			keys = append(keys, key)
		}
		values[key] = value
	}
	if _, err := decoder.ReadToken(); err != nil {
		return nil, err
	}
	out = append(out, '{')
	for i, key := range keys {
		if i > 0 {
			out = append(out, ',')
		}
		out = AppendJSONString(out, key)
		out = append(out, ':')
		out = append(out, values[key]...)
	}
	return append(out, '}'), nil
}

// appendNumber appends the number text as serde_json prints what it reads.
func appendNumber(out []byte, text string) ([]byte, error) {
	if !strings.ContainsAny(text, ".eE") {
		if strings.HasPrefix(text, "-") {
			if n, err := strconv.ParseInt(text, 10, 64); err == nil {
				return strconv.AppendInt(out, n, 10), nil
			}
		} else if n, err := strconv.ParseUint(text, 10, 64); err == nil {
			return strconv.AppendUint(out, n, 10), nil
		}
	}
	f, err := strconv.ParseFloat(text, 64)
	if err != nil && (math.IsInf(f, 0) || !errors.Is(err, strconv.ErrRange)) {
		return nil, errors.New("number out of range")
	}
	return append(out, JSONFloat(f)...), nil
}

// AppendJSONString appends s as a JSON string as serde_json writes it: a quote, a
// backslash and the control characters escaped, by name where JSON has one, and
// nothing else.
func AppendJSONString(out []byte, s string) []byte {
	const hex = "0123456789abcdef"
	out = append(out, '"')
	for _, r := range s {
		switch {
		case r == '"':
			out = append(out, `\"`...)
		case r == '\\':
			out = append(out, `\\`...)
		case r == '\b':
			out = append(out, `\b`...)
		case r == '\f':
			out = append(out, `\f`...)
		case r == '\n':
			out = append(out, `\n`...)
		case r == '\r':
			out = append(out, `\r`...)
		case r == '\t':
			out = append(out, `\t`...)
		case r < 0x20:
			out = append(out, '\\', 'u', '0', '0', hex[r>>4], hex[r&0xf])
		default:
			out = append(out, string(r)...)
		}
	}
	return append(out, '"')
}
