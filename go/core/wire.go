// Package core holds shared contract data and pure lookups for the browser,
// agent and backend. Its wire conventions follow docs/architecture/contracts.md.
package core

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"

	"github.com/wspl/demi/go/internal/wire"
)

const MaxSafeInteger uint64 = 1<<53 - 1

type InvalidError = wire.InvalidError

// Decode checks one JSON document at the boundary that receives a core type.
func Decode[T any](data []byte) (T, error) {
	var value T
	if err := json.Unmarshal(data, &value, wireOptions); err != nil {
		kind := DecodeShape
		var syntax *jsontext.SyntacticError
		if errors.As(err, &syntax) && !errors.Is(err, jsontext.ErrDuplicateName) {
			kind = DecodeSyntax
		}
		var zero T
		return zero, &DecodeError{Kind: kind, Err: wire.Refusal(err)}
	}
	if err := check(value); err != nil {
		var zero T
		return zero, &DecodeError{Kind: DecodeInvalid, Err: err}
	}
	return value, nil
}

// DecodeValue checks an already parsed JSON value against a core contract.
func DecodeValue[T any](value jsontext.Value) (T, error) { return Decode[T](value) }

// Encode checks a core value before writing its deterministic JSON encoding.
func Encode[T any](value T) ([]byte, error) { return encode(value) }

// Validate checks a core value, including values held by another package.
func Validate[T any](value T) error { return check(value) }

var ErrEmptyID = errors.New("an identity must not be empty")

// Why a value that entered the process was refused.
type DecodeError struct {
	Kind DecodeErrorKind
	Err  error
}
type DecodeErrorKind string

const (
	DecodeSyntax  DecodeErrorKind = "syntax"
	DecodeShape   DecodeErrorKind = "shape"
	DecodeInvalid DecodeErrorKind = "invalid"
)

func (e *DecodeError) Error() string {
	if e.Kind == DecodeSyntax {
		return "not JSON: " + e.Err.Error()
	}
	return e.Err.Error()
}
func (e *DecodeError) Unwrap() error { return e.Err }
