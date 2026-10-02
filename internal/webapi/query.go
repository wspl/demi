package webapi

import "fmt"

// A boolean query parameter: spelled exactly `true` or `false`, and false
// when omitted. Anything else, such as `1`, `TRUE` or an empty value, is
// refused rather than read as one of them.
// +demi:root
type StrictBool bool

// ParseStrictBool reads the exact spelling of a boolean query parameter.
func ParseStrictBool(text string) (StrictBool, error) {
	switch text {
	case "true":
		return true, nil
	case "false":
		return false, nil
	default:
		return false, fmt.Errorf("must be true or false, not %q", text)
	}
}

// `?refresh=true|false`.
// +demi:root
// +demi:tolerant
type Refresh struct {
	Refresh StrictBool `json:"refresh,omitempty"`
}
