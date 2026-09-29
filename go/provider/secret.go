package provider

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"unicode"

	"github.com/wspl/demi/go/internal/wire"
)

// Secret holds a credential. Formatting never exposes it; Expose is explicit.
//
//demi:opaque
type Secret struct{ text string }

func NewSecret(text string) (Secret, error) {
	if text == "" {
		return Secret{}, errors.New("the credential is empty")
	}
	for _, r := range text {
		if unicode.IsControl(r) {
			return Secret{}, errors.New("the credential contains a control character")
		}
	}
	return Secret{text: text}, nil
}
func (s Secret) Expose() string                { return s.text }
func (s Secret) String() string                { return "Secret(..)" }
func (s Secret) GoString() string              { return "Secret(..)" }
func (s Secret) Format(f fmt.State, verb rune) { fmt.Fprint(f, "Secret(..)") }
func (s Secret) Bearer() string                { return "Bearer " + s.text }
func (s Secret) MarshalJSONTo(enc *jsontext.Encoder) error {
	if _, err := NewSecret(s.text); err != nil {
		return err
	}
	return json.MarshalEncode(enc, s.text)
}
func (s *Secret) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	next, err := NewSecret(text)
	if err == nil {
		*s = next
	}
	return err
}

func (s Secret) validate() error { _, err := NewSecret(s.text); return err }
