package backend

import (
	"crypto/aes"
	"crypto/cipher"
	"encoding/binary"
	"errors"

	"github.com/wspl/demi/go/webapi"
)

// ErrUnsealable is a sealed value that does not open: it was altered, moved
// to another row, or sealed under another instance secret. Nothing replaces
// it.
var ErrUnsealable = errors.New("the sealed value does not open")

// VaultKey is the key credentials are sealed under (storage.md § Passwords
// and credentials at rest): a document encrypted with AES-256-GCM, bound to
// the row it belongs to, and stored as one BLOB of a random 12-byte nonce,
// the ciphertext and the 16-byte tag.
type VaultKey struct{ aead cipher.AEAD }

// NewVaultKey makes the vault's key from 32 bytes.
func NewVaultKey(key []byte) (VaultKey, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return VaultKey{}, err
	}
	aead, err := cipher.NewGCMWithRandomNonce(block)
	if err != nil {
		return VaultKey{}, err
	}
	return VaultKey{aead}, nil
}

// Seal returns plaintext sealed for row.
func (k VaultKey) Seal(row SealedRow, plaintext []byte) []byte {
	return k.aead.Seal(nil, nil, plaintext, row.name())
}

// Open returns the plaintext sealed holds for row, or ErrUnsealable.
func (k VaultKey) Open(row SealedRow, sealed []byte) ([]byte, error) {
	plaintext, err := k.aead.Open(nil, nil, sealed, row.name())
	if err != nil {
		return nil, ErrUnsealable
	}
	return plaintext, nil
}

// SealedRow is the row a sealed value belongs to: its additional
// authenticated data, so that a value copied into another row does not open.
type SealedRow struct {
	label string
	ids   []string
}

// ConfigRow is the row of API-key entry provider's configuration.
func ConfigRow(provider webapi.ProviderID) SealedRow {
	return SealedRow{"demi provider config", []string{provider.String()}}
}

// SecretRow is the row of a subscription account's secret document.
func SecretRow(provider webapi.ProviderID, account webapi.CredentialID) SealedRow {
	return SealedRow{"demi account secret", []string{provider.String(), account.String()}}
}

// name is the row's name as bytes: its label, then each id preceded by its
// length as four big-endian bytes, so that no two rows share a name.
func (r SealedRow) name() []byte {
	name := []byte(r.label)
	for _, id := range r.ids {
		name = binary.BigEndian.AppendUint32(name, uint32(len(id)))
		name = append(name, id...)
	}
	return name
}
