// Package wire is what the decoders and rule checks that cmd/wiregen writes
// share: the error a refusal is, the path of the field it names, the reads of
// the JSON kinds and the checks of the rules. Nothing here reflects on types.
package wire

import (
	"errors"
	"strconv"
)

// An InvalidError says that a value from outside the process, or one about to
// leave it, breaks the rules of its wire type. It names the field and the rule,
// never the value: a value may be a secret, and errors are logged.
type InvalidError struct {
	// Path is the field the rule is about: members joined with ".", indexes as
	// [3] and map keys as ["NAME"]. It is empty for the value itself.
	Path string
	// Rule says what the field breaks.
	Rule string
}

func (e *InvalidError) Error() string {
	if e.Path == "" {
		return "invalid: " + e.Rule
	}
	return "invalid: " + e.Path + ": " + e.Rule
}

// Index returns the path element of the element of a slice at i.
func Index(i int) string {
	return "[" + strconv.Itoa(i) + "]"
}

// Key returns the path element of the entry of a map under key. A key is a name
// the contract chose, never a secret value.
func Key(key string) string {
	return "[" + strconv.Quote(key) + "]"
}

// Prefix returns the path of the field path of a value that is the field elem of
// its parent.
func Prefix(elem, path string) string {
	switch {
	case elem == "":
		return path
	case path == "":
		return elem
	case path[0] == '[':
		return elem + path
	default:
		return elem + "." + path
	}
}

// In returns err, which refused the field elem of a value, as a refusal of the
// value: an *InvalidError gets elem in front of its path, and an error of the
// JSON decoder becomes a refusal at elem.
func In(elem string, err error) error {
	var invalid *InvalidError
	if errors.As(err, &invalid) {
		return &InvalidError{Path: Prefix(elem, invalid.Path), Rule: invalid.Rule}
	}
	return &InvalidError{Path: elem, Rule: syntaxRule(err)}
}
