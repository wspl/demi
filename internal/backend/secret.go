package backend

import (
	"context"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/usershard"
)

// InstanceSecret is the instance secret. Formatting never shows its bytes.
type InstanceSecret struct{ bytes [32]byte }

// ParseInstanceSecret reads exactly 64 hexadecimal digits. Its error never
// includes the supplied text.
func ParseInstanceSecret(text string) (InstanceSecret, error) {
	var secret InstanceSecret
	if len(text) != 64 {
		return secret, &SecretError{Kind: SecretMalformed}
	}
	if _, err := hex.Decode(secret.bytes[:], []byte(text)); err != nil {
		return InstanceSecret{}, &SecretError{Kind: SecretMalformed}
	}
	return secret, nil
}

// String hides the secret in ordinary formatting.
func (s InstanceSecret) String() string { return "InstanceSecret(..)" }

// GoString hides the secret in Go-syntax formatting.
func (s InstanceSecret) GoString() string { return s.String() }

// SecretErrorKind identifies why the instance secret is unusable.
type SecretErrorKind uint8

const (
	// SecretMalformed means the configured secret is not 64 hexadecimal digits.
	SecretMalformed SecretErrorKind = iota
	// SecretCorrupt means the persisted secret is not 64 hexadecimal digits.
	SecretCorrupt
	// SecretRead means the persisted secret could not be read.
	SecretRead
	// SecretCreate means the first-use secret could not be published.
	SecretCreate
)

// SecretError is why the instance secret is unusable. A malformed secret
// stops startup, and no error shows the secret itself.
type SecretError struct {
	// Kind identifies the failed operation.
	Kind SecretErrorKind
	// Path names the path involved in the failed operation.
	Path string
	// Err is the underlying failure, when present.
	Err error
}

// Error describes the failure without revealing the secret.
func (e *SecretError) Error() string {
	switch e.Kind {
	case SecretMalformed:
		return "the instance secret must be 64 hexadecimal digits"
	case SecretCorrupt:
		return fmt.Sprintf("the instance secret file %s is not 64 hexadecimal digits", e.Path)
	case SecretRead:
		return fmt.Sprintf("the instance secret file %s cannot be read: %v", e.Path, e.Err)
	case SecretCreate:
		return fmt.Sprintf("the instance secret file %s cannot be created: %v", e.Path, e.Err)
	}
	return fmt.Sprint(e.Err)
}

// Unwrap returns the underlying read or publication error.
func (e *SecretError) Unwrap() error { return e.Err }

// loadSecret reads or atomically publishes the backend instance secret.
func loadSecret(ctx context.Context, directory string) (InstanceSecret, error) {
	path := filepath.Join(directory, "instance-secret")
	data, err := os.ReadFile(path)
	if err == nil {
		if !utf8.Valid(data) {
			return InstanceSecret{}, &SecretError{
				Kind: SecretRead,
				Path: path,
				Err:  errors.New("stream did not contain valid UTF-8"),
			}
		}
		secret, parseErr := ParseInstanceSecret(strings.TrimRightFunc(string(data), unicode.IsSpace))
		if parseErr != nil {
			return InstanceSecret{}, &SecretError{Kind: SecretCorrupt, Path: path}
		}
		return secret, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return InstanceSecret{}, &SecretError{Kind: SecretRead, Path: path, Err: err}
	}
	var secret InstanceSecret
	if _, err := rand.Read(secret.bytes[:]); err != nil {
		return secret, &SecretError{Kind: SecretCreate, Path: path, Err: err}
	}
	text := hex.EncodeToString(secret.bytes[:]) + "\n"
	if err := artifacts.PublishBytes(
		ctx,
		path,
		[]byte(text),
		artifacts.Publication{Mode: artifacts.CreateNew, Permissions: artifacts.Private, Durable: true},
	); err != nil {
		return InstanceSecret{}, &SecretError{Kind: SecretCreate, Path: path, Err: err}
	}
	return secret, nil
}

// serviceKeys derives the backend's two independently labeled instance keys.
func (s InstanceSecret) serviceKeys() (usershard.ServiceKeys, error) {
	email, err := hkdf.Key(sha256.New, s.bytes[:], nil, "demi email-change code", 32)
	if err != nil {
		return usershard.ServiceKeys{}, fmt.Errorf("derive email code key: %w", err)
	}
	vault, err := hkdf.Key(sha256.New, s.bytes[:], nil, "demi provider vault", 32)
	if err != nil {
		return usershard.ServiceKeys{}, fmt.Errorf("derive provider vault key: %w", err)
	}
	return usershard.ServiceKeys{
		EmailCodes: accounts.NewCodeKey([32]byte(email)),
		Vault:      *providers.NewVaultKey([32]byte(vault)),
	}, nil
}
