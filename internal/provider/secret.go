package provider

import (
	"errors"
	"fmt"
	"unicode"
	"unicode/utf8"
)

// A credential, such as an API key or a setup token: nonempty, one line of
// text without control characters, and never printed. `Debug` shows
// `Secret(..)`, so a logged configuration cannot leak it.
// +demi:root
// +demi:length chars min=1
// +demi:check validateSecret
//
//nolint:revive // Contract documentation is product text copied verbatim from Rust.
type Secret string

// ErrSecretEmpty means the credential is empty.
var ErrSecretEmpty = errors.New("the credential is empty")

// ErrSecretControl means the credential contains a control character.
var ErrSecretControl = errors.New("the credential contains a control character")

// ErrSecretUTF8 means the credential is not valid UTF-8 text.
var ErrSecretUTF8 = errors.New("the credential is not UTF-8 text")

// NewSecret validates a credential without including its text in errors.
func NewSecret(text string) (Secret, error) {
	secret := Secret(text)
	if err := validateSecret(secret); err != nil {
		return "", err
	}
	return secret, nil
}
func validateSecret(secret Secret) error {
	if secret == "" {
		return ErrSecretEmpty
	}
	if !utf8.ValidString(string(secret)) {
		return ErrSecretUTF8
	}
	for _, r := range secret {
		if unicode.IsControl(r) {
			return ErrSecretControl
		}
	}
	return nil
}

// Expose explicitly reveals the credential for its intended recipient.
func (s Secret) Expose() string { return string(s) }

// Format redacts all fmt verbs, including %#v.
func (s Secret) Format(f fmt.State, _ rune) { _, _ = f.Write([]byte("Secret(..)")) }

// HeaderValue returns a redacted header value whose Expose method supplies HTTP text.
func (s Secret) HeaderValue() Secret { return s }

// Bearer returns a redacted bearer authorization value.
func (s Secret) Bearer() Secret { return Secret("Bearer " + string(s)) }
