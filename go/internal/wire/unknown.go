package wire

import (
	"encoding/json/jsontext"
	"slices"
	"unicode/utf8"
)

// CheckUnknown prevents retained members from shadowing declared wire names.
func CheckUnknown(values map[string]jsontext.Value, reserved ...string) error {
	for key := range values {
		if !utf8.ValidString(key) {
			return &InvalidError{Rule: "invalid UTF-8 member name"}
		}
		if slices.Contains(reserved, key) {
			return &InvalidError{Path: key, Rule: "retained member shadows a declared member"}
		}
	}
	return nil
}
