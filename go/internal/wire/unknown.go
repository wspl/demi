package wire

import (
	"bytes"
	"encoding/json/jsontext"
	"slices"
	"unicode/utf8"
)

// Member is a retained JSON-valued wire member.
type Member struct {
	Name  string
	Value jsontext.Value
}

// Members keeps unknown members in arrival order for replay.
type Members []Member

// JSON returns the retained members as an object without changing their order.
func (values Members) JSON() (jsontext.Value, error) {
	var data bytes.Buffer
	enc := jsontext.NewEncoder(&data)
	if err := enc.WriteToken(jsontext.BeginObject); err != nil {
		return nil, Refusal(err)
	}
	for _, member := range values {
		if err := enc.WriteToken(jsontext.String(member.Name)); err != nil {
			return nil, Refusal(err)
		}
		if err := enc.WriteValue(member.Value); err != nil {
			return nil, In(member.Name, err)
		}
	}
	if err := enc.WriteToken(jsontext.EndObject); err != nil {
		return nil, Refusal(err)
	}
	return jsontext.Value(data.Bytes()), nil
}

// CheckUnknown prevents duplicate retained names and shadows of declared names.
func CheckUnknown(values Members, reserved ...string) error {
	seen := make(map[string]bool, len(values))
	for _, member := range values {
		key := member.Name
		if !utf8.ValidString(key) {
			return &InvalidError{Rule: "invalid UTF-8 member name"}
		}
		if slices.Contains(reserved, key) {
			return &InvalidError{Path: key, Rule: "retained member shadows a declared member"}
		}
		if seen[key] {
			return &InvalidError{Path: key, Rule: "duplicate member"}
		}
		seen[key] = true
	}
	return nil
}
