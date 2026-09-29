package provider

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"

	"github.com/wspl/demi/go/internal/wire"
)

//demi:wire open
type vendorTag struct {
	Type string `json:"type"`
}

// DecodeTagged reads a vendor tag before invoking its registered boundary
// decoder. Unknown tags are absent; a missing or malformed tag is an error.
func DecodeTagged[T any](data []byte, registered map[string]func([]byte) (T, error)) (T, bool, error) {
	var zero T
	tag, err := decode[vendorTag](data)
	if err != nil {
		return zero, false, err
	}
	decodePayload, ok := registered[tag.Type]
	if !ok {
		return zero, false, nil
	}
	payload, err := decodePayload(data)
	return payload, err == nil, err
}

// ReportedString is display-only vendor text; another shape reads as absent.
//
//demi:opaque
type ReportedString struct{ Value *string }

func (s *ReportedString) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	value, err := dec.ReadValue()
	if err != nil {
		return err
	}
	s.Value = nil
	if value.Kind() == '"' {
		var text string
		if err := json.Unmarshal(value, &text); err != nil {
			return err
		}
		s.Value = &text
	}
	return nil
}

// SecretDecodeError deliberately excludes decoder messages and values.
type SecretDecodeError struct {
	Path  string
	Fault string
}

func (e *SecretDecodeError) Error() string {
	return fmt.Sprintf("malformed at %s: %s", e.Path, e.Fault)
}

// DecodeSecret applies a family's boundary decoder and reports only a path
// and fault category, never a token-bearing error message.
func DecodeSecret[T any](data []byte, boundary func([]byte) (T, error)) (T, error) {
	value, err := boundary(data)
	if err == nil {
		return value, nil
	}
	fault := "a field is missing, unknown or of the wrong type"
	path := "."
	var invalid *wire.InvalidError
	var syntax *jsontext.SyntacticError
	if errors.As(err, &syntax) {
		fault = "not JSON"
	}
	if errors.As(err, &invalid) {
		if invalid.Rule == "is not valid JSON" {
			fault = "not JSON"
		} else if invalid.Path != "" && invalid.Rule != "required" {
			path = invalid.Path
		}
	}
	var zero T
	return zero, &SecretDecodeError{Path: path, Fault: fault}
}

func (ReportedString) validate() error { return nil }
