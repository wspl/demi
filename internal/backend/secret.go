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
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/usershard"
)

// InstanceSecret is the instance secret. Formatting never shows its bytes.
type InstanceSecret struct{ bytes [32]byte }

// ParseInstanceSecret reads exactly 64 hexadecimal digits. Its error never
// includes the supplied text.
func ParseInstanceSecret(text string) (InstanceSecret, error) {
	var secret InstanceSecret
	if len(text) != 64 {
		return InstanceSecret{}, errMalformedSecret
	}
	if _, err := hex.Decode(secret.bytes[:], []byte(text)); err != nil {
		return InstanceSecret{}, errMalformedSecret
	}
	return secret, nil
}

// String hides the secret in ordinary formatting.
func (s InstanceSecret) String() string {
	return "InstanceSecret(..)"
}

// GoString hides the secret in Go-syntax formatting.
func (s InstanceSecret) GoString() string {
	return s.String()
}

var errMalformedSecret = errors.New("the instance secret must be 64 hexadecimal digits")

// loadSecret reads or atomically publishes the backend instance secret.
func loadSecret(ctx context.Context, directory string) (InstanceSecret, error) {
	path := filepath.Join(directory, "instance-secret")
	data, err := os.ReadFile(path)
	if err == nil {
		if !utf8.Valid(data) {
			return InstanceSecret{}, fmt.Errorf(
				"the instance secret file %s cannot be read: stream did not contain valid UTF-8",
				path,
			)
		}
		secret, parseErr := ParseInstanceSecret(strings.TrimRightFunc(string(data), unicode.IsSpace))
		if parseErr != nil {
			return InstanceSecret{}, fmt.Errorf("the instance secret file %s is not 64 hexadecimal digits", path)
		}
		return secret, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return InstanceSecret{}, fmt.Errorf("the instance secret file %s cannot be read: %w", path, err)
	}
	var secret InstanceSecret
	if _, err := rand.Read(secret.bytes[:]); err != nil {
		return secret, fmt.Errorf("the instance secret file %s cannot be created: %w", path, err)
	}
	text := hex.EncodeToString(secret.bytes[:]) + "\n"
	if err := artifacts.PublishBytes(
		ctx,
		path,
		[]byte(text),
		artifacts.Publication{Mode: artifacts.CreateNew, Permissions: artifacts.Private, Durable: true},
	); err != nil {
		return InstanceSecret{}, fmt.Errorf("the instance secret file %s cannot be created: %w", path, err)
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
		Vault:      *providerhost.NewVaultKey([32]byte(vault)),
	}, nil
}
