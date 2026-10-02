//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"github.com/wspl/demi/internal/webapi"
)

// VaultKey is the key credentials are sealed under, using AES-256-GCM.
type VaultKey struct{}

// Row binds a sealed value to its storage row through authenticated data.
type Row interface{ sealedRow() }

// ConfigRow is an API-key entry configuration row.
type ConfigRow struct{ Provider webapi.ProviderID }

func (ConfigRow) sealedRow() {}

// SecretRow is a subscription account secret document row.
type SecretRow struct {
	Provider webapi.ProviderID
	Account  webapi.CredentialID
}

func (SecretRow) sealedRow() {}

// NewVaultKey creates a credential encryption key.
func NewVaultKey(key [32]byte) *VaultKey { panic("not written: b-providers") }

// Seal encrypts plaintext and binds it to its row.
func (k *VaultKey) Seal(row Row, plaintext []byte) ([]byte, error) { panic("not written: b-providers") }

// Open authenticates and decrypts a sealed row; corrupt values are never repaired.
func (k *VaultKey) Open(row Row, sealed []byte) ([]byte, error) { panic("not written: b-providers") }
