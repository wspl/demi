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

// +demi:root direction=receive output=plugin-schemacheck
// +demi:schema
type PlainRange struct {
	// +demi:range min=1 max=10
	Value float64 `json:"value"`
}

// +demi:root direction=receive output=plugin-schemacheck
// +demi:schema
type SchemaRange struct {
	// +demi:range min=1 max=10 schema-only
	Value float64 `json:"value"`
}

// +demi:range min=1 max=10 schema-only
// +demi:check validateNumber
type CheckedNumber float64

func validateNumber(value CheckedNumber) error {
	if value != 5 {
		return ErrReserved
	}
	return nil
}

// +demi:check validateStdin
type Stdin string

func validateStdin(value Stdin) error {
	if value == "" {
		return errors.New("must not be empty; use shell_status to poll")
	}
	return nil
}

// +demi:root direction=receive output=plugin-schemacheck
// +demi:schema
type CheckedInput struct {
	Number CheckedNumber `json:"number"`
	Stdin  Stdin         `json:"stdin"`
}
