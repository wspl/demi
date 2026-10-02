package claudecode

import (
	"encoding/json"

	"github.com/wspl/demi/internal/provider"
)

// serdeValue uses the shared vendor decoder's serde Value representation:
// object insertion order, compact whitespace, scalar spelling and escaping.
// RawMessage accepts every JSON shape, so Reported cannot discard a valid value.
type serdeValue json.RawMessage

func (v serdeValue) MarshalJSON() ([]byte, error) {
	value, err := provider.DecodeUntagged[provider.Reported[json.RawMessage]](string(v))
	if err != nil {
		return nil, err
	}
	return provider.JSONBody(*value.Value)
}
