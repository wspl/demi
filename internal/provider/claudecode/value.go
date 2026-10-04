package claudecode

import (
	"encoding/json"

	"github.com/wspl/demi/internal/provider"
)

// canonicalJSON is a JSON value that encodes in the vendor decoder's canonical
// form: object members in read order (a repeated member keeps its last value),
// compact whitespace, integers as integers and other numbers as floats that
// keep ".0", and strings without escaping <, >, &, U+2028 or U+2029.
// RawMessage accepts every JSON shape, so Reported cannot discard a valid value.
type canonicalJSON json.RawMessage

// MarshalJSON encodes the value in canonical form.
func (v canonicalJSON) MarshalJSON() ([]byte, error) {
	value, err := provider.DecodeUntagged[provider.Reported[json.RawMessage]](string(v))
	if err != nil {
		return nil, err
	}
	return provider.JSONBody(*value.Value)
}
