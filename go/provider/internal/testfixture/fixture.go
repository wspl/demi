// Package testfixture holds generated boundary types for provider protocol tests.
package testfixture

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

//demi:wire
type Tokens struct {
	Access  string `json:"access"`
	Refresh string `json:"refresh"`
}

func DecodeTokens(data []byte) (Tokens, error)   { return decode[Tokens](data) }
func EncodeTokens(tokens Tokens) ([]byte, error) { return encode(tokens) }

//demi:wire open
type Event struct {
	Index uint32 `json:"index"`
	Text  string `json:"text"`
}

func DecodeEvent(data []byte) (Event, error) { return decode[Event](data) }
