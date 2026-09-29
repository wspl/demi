package core

import (
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/internal/wire"
)

// IdentityKind supplies the rule for a family's identities. Implementations
// must be stateless and usable as their zero value.
type IdentityKind interface{ CheckIdentity(string) error }

// Identity is checked string storage, the Go counterpart of Rust's id! macro.
// A package declares its own opaque wrapper embedding Identity[ItsKind]. All
// creation and decoding runs ItsKind.CheckIdentity; encoding rejects zero IDs.
type Identity[K IdentityKind] struct{ text string }

func ParseIdentity[K IdentityKind](text string) (Identity[K], error) {
	var kind K
	if err := kind.CheckIdentity(text); err != nil {
		return Identity[K]{}, err
	}
	return Identity[K]{text: text}, nil
}

// NonemptyIdentity is the rule for an identity compared exactly as supplied.
type NonemptyIdentity struct{}

func (NonemptyIdentity) CheckIdentity(text string) error {
	if text == "" {
		return ErrEmptyID
	}
	return nil
}

func (v Identity[K]) String() string { return v.text }
func (v Identity[K]) validate() error {
	_, err := ParseIdentity[K](v.text)
	if err != nil {
		return &InvalidError{Rule: err.Error()}
	}
	return nil
}
func (v Identity[K]) MarshalJSONTo(enc *jsontext.Encoder) error {
	if err := v.validate(); err != nil {
		return err
	}
	return enc.WriteToken(jsontext.String(v.text))
}
func (v *Identity[K]) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	parsed, err := ParseIdentity[K](text)
	if err != nil {
		return &InvalidError{Rule: err.Error()}
	}
	*v = parsed
	return nil
}
func (v Identity[K]) MarshalText() ([]byte, error) {
	if err := v.validate(); err != nil {
		return nil, err
	}
	return []byte(v.text), nil
}
func (v *Identity[K]) UnmarshalText(text []byte) error {
	parsed, err := ParseIdentity[K](string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}
