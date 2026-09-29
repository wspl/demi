// Package provider owns the inference provider contract and shared vendor protocols.
package provider

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

// Validate checks a declared provider wire value held by another package.
func Validate[T any](value T) error { return check(value) }
