// Package schemacheck exercises rules enforced only by Go on schema roots.
package schemacheck

import "errors"

//go:generate go run ../.. .

var ErrReserved = errors.New("reserved value")

// +demi:root direction=receive output=plugin-schemacheck
// +demi:schema
// +demi:length chars min=1
// +demi:check validateText
type CheckedText string

func validateText(value CheckedText) error {
	if value == "reserved" {
		return ErrReserved
	}
	return nil
}

// +demi:root direction=receive output=plugin-schemacheck
// +demi:schema
type Envelope struct{ CheckedObject }

// +demi:check validateObject
type CheckedObject struct {
	Value string `json:"value"`
}

func validateObject(value CheckedObject) error {
	if value.Value == "reserved" {
		return ErrReserved
	}
	return nil
}
