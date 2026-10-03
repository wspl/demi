package providers

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/binary"
	"fmt"

	"github.com/wspl/demi/internal/webapi"
)

// VaultKey is the key credentials are sealed under, using AES-256-GCM.
type VaultKey struct{ key [32]byte }

// Row binds a sealed value to its storage row through authenticated data.
type Row interface{ sealedRow() }

// ConfigRow is an API-key entry configuration row.
type ConfigRow struct {
	// Provider identifies the provider entry bound to this record.
	Provider webapi.ProviderID
}

func (ConfigRow) sealedRow() {}

// SecretRow is a subscription account secret document row.
type SecretRow struct {
	// Provider identifies the provider entry bound to this record.
	Provider webapi.ProviderID
	// Account identifies the subscription account bound to this provider.
	Account webapi.CredentialID
}

func (SecretRow) sealedRow() {}

// NewVaultKey creates a credential encryption key.
func NewVaultKey(key [32]byte) *VaultKey {
	return &VaultKey{key: key}
}

// Seal encrypts plaintext and binds it to its row.
func (k *VaultKey) Seal(row Row, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(k.key[:])
	if err != nil {
		return nil, fmt.Errorf("create vault cipher: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("create vault cipher: %w", err)
	}
	aad, err := rowName(row)
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, gcm.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return nil, fmt.Errorf("create vault nonce: %w", err)
	}
	return gcm.Seal(nonce, nonce, plaintext, aad), nil
}

// Open authenticates and decrypts a sealed row; corrupt values are never repaired.
func (k *VaultKey) Open(row Row, sealed []byte) ([]byte, error) {
	if len(sealed) < 12 {
		return nil, &Unsealable{}
	}
	block, err := aes.NewCipher(k.key[:])
	if err != nil {
		return nil, &Unsealable{}
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, &Unsealable{}
	}
	aad, err := rowName(row)
	if err != nil {
		return nil, &Unsealable{}
	}
	plain, err := gcm.Open(nil, sealed[:12], sealed[12:], aad)
	if err != nil {
		return nil, &Unsealable{}
	}
	return plain, nil
}

// rowName binds each credential ciphertext to its Rust storage identity.
func rowName(row Row) ([]byte, error) {
	var name []byte
	var ids []string
	switch row := row.(type) {
	case ConfigRow:
		name = []byte("demi provider config")
		ids = []string{string(row.Provider)}
	case SecretRow:
		name = []byte("demi account secret")
		ids = []string{string(row.Provider), string(row.Account)}
	default:
		return nil, &Unsealable{}
	}
	for _, id := range ids {
		length := uint32(len(id))
		name = binary.BigEndian.AppendUint32(name, length)
		name = append(name, id...)
	}
	return name, nil
}
