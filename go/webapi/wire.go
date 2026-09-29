// Package webapi declares the browser's REST request and response contracts.
package webapi

//go:generate go run ./internal/generate

import (
	"encoding/json/v2"
	"github.com/wspl/demi/go/internal/wire"
)

type InvalidError = wire.InvalidError

func Decode[T any](data []byte) (T, error) {
	var value T
	if err := json.Unmarshal(data, &value, wireOptions); err != nil {
		return value, wire.Refusal(err)
	}
	return value, check(value)
}
func Encode[T any](value T) ([]byte, error) {
	if err := check(value); err != nil {
		return nil, err
	}
	return json.Marshal(value, wireOptions)
}
func Validate[T any](value T) error { return check(value) }
