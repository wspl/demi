package backend

import (
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"unicode"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/internal/fsfail"
)

// instanceSecretFile is the data directory's file that holds the instance
// secret when DEMI_INSTANCE_SECRET is not set.
const instanceSecretFile = "instance-secret"

// The HKDF labels of the keys derived from the instance secret; no two keys
// share one.
const (
	emailCodeLabel = "demi email-change code"
	vaultLabel     = "demi provider vault"
)

// ErrMalformedSecret refuses a configured instance secret that is not 64
// hexadecimal digits.
var ErrMalformedSecret = errors.New("the instance secret must be 64 hexadecimal digits")

// InstanceSecret is the instance secret (storage.md § Passwords and
// credentials at rest): 32 random bytes, DEMI_INSTANCE_SECRET when that is
// set, otherwise the data directory's instance-secret file, which the first
// start creates readable only by its owner. Each key the backend needs
// derives from it with HKDF-SHA256 under a label of its own. Formatting it
// never shows it, and no error about it does.
type InstanceSecret struct{ bytes [32]byte }

// ParseInstanceSecret reads a secret of 64 hexadecimal digits, in either
// case.
func ParseInstanceSecret(text string) (InstanceSecret, error) {
	var secret InstanceSecret
	if hex.DecodedLen(len(text)) != len(secret.bytes) {
		return InstanceSecret{}, ErrMalformedSecret
	}
	if _, err := hex.Decode(secret.bytes[:], []byte(text)); err != nil {
		return InstanceSecret{}, ErrMalformedSecret
	}
	return secret, nil
}

// LoadInstanceSecret reads the secret of the data directory dataDir, and
// creates it on first use.
func LoadInstanceSecret(dataDir string) (InstanceSecret, error) {
	path := filepath.Join(dataDir, instanceSecretFile)
	text, err := os.ReadFile(path)
	if err == nil {
		secret, err := ParseInstanceSecret(strings.TrimRightFunc(string(text), unicode.IsSpace))
		if err != nil {
			return InstanceSecret{}, fmt.Errorf("the instance secret file %s is not 64 hexadecimal digits", path)
		}
		return secret, nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return InstanceSecret{}, fmt.Errorf("the instance secret file %s cannot be read: %w", path, fsfail.Cause(err))
	}
	var secret InstanceSecret
	// crypto/rand never fails: it crashes the program instead.
	rand.Read(secret.bytes[:])
	publication := artifact.Publication{Mode: artifact.CreateNew, Permissions: artifact.Private, Durable: true}
	if err := artifact.PublishBytes(path, []byte(hex.EncodeToString(secret.bytes[:])+"\n"), publication); err != nil {
		return InstanceSecret{}, fmt.Errorf("the instance secret file %s cannot be created: %w", path, fsfail.Cause(err))
	}
	return secret, nil
}

// Format writes a placeholder for every verb: the secret never shows.
func (InstanceSecret) Format(f fmt.State, _ rune) { fmt.Fprint(f, "InstanceSecret(..)") }

// EmailCodeKey is the key email-change codes are hashed under.
func (s InstanceSecret) EmailCodeKey() []byte { return s.subkey(emailCodeLabel) }

// VaultKey is the key provider credentials are sealed under.
func (s InstanceSecret) VaultKey() VaultKey {
	key, err := NewVaultKey(s.subkey(vaultLabel))
	if err != nil {
		// A 32-byte key always makes an AES-256 cipher.
		panic(err)
	}
	return key
}

// subkey is the HKDF-SHA256 key under label, with no salt.
func (s InstanceSecret) subkey(label string) []byte {
	key, err := hkdf.Key(sha256.New, s.bytes[:], nil, label, 32)
	if err != nil {
		// 32 bytes is a valid HKDF-SHA256 output length.
		panic(err)
	}
	return key
}
