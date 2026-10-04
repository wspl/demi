package backend

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/providerhost"
)

// TestInstanceSecretKeysAndCorruptFile checks key derivation, redaction and
// the refusal of a corrupt secret file. Local files only, without starting the
// backend or a vendor; persistence across restarts and the file's permissions
// are checked through backend.Start by TestInstanceSecretPersistsWithPrivatePermissions.
func TestInstanceSecretKeysAndCorruptFile(t *testing.T) {
	t.Parallel()
	directory := t.TempDir()
	path := filepath.Join(directory, "instance-secret")
	secret, err := ParseInstanceSecret("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
	if err != nil {
		t.Fatal(err)
	}
	keys, err := secret.serviceKeys()
	if err != nil {
		t.Fatal(err)
	}
	// Independent HMAC-SHA256 extract/expand vectors, with a zero salt and the
	// labels "demi email-change code" and "demi provider vault" (one block,
	// suffixed with byte 1).
	if hex.EncodeToString(keys.EmailCodes[:]) != "d02e61f8c078484c43cdfbf10bb81f1742a335a7871526b189ce947e202103ca" {
		t.Fatal("email key differs from HKDF vector")
	}
	vaultBytes, err := hex.DecodeString("5a76cb6d56804a2c62f3b8f8558930481d501c87730c7bbe7966db31c6057d43")
	if err != nil {
		t.Fatal(err)
	}
	expected := providerhost.NewVaultKey([32]byte(vaultBytes))
	row := providerhost.ConfigRow{Provider: "provider-1"}
	encrypted, err := keys.Vault.Seal(row, []byte("credential"))
	if err != nil {
		t.Fatal(err)
	}
	plaintext, err := expected.Open(row, encrypted)
	if err != nil || !bytes.Equal(plaintext, []byte("credential")) {
		t.Fatalf("vault key differs from HKDF vector: %v", err)
	}
	for _, format := range []string{"%v", "%+v", "%#v", "%s"} {
		if got := fmt.Sprintf(format, secret); got != "InstanceSecret(..)" {
			t.Fatalf("secret formatting exposed data with %s", format)
		}
	}
	for _, scenario := range []struct {
		name, value string
		want        string
	}{
		{"malformed", "not-a-secret", "the instance secret file " + path + " is not 64 hexadecimal digits"},
		{
			"invalid UTF8",
			string([]byte{0xff}),
			"the instance secret file " + path + " cannot be read: stream did not contain valid UTF-8",
		},
		{
			"leading space",
			" " + strings.Repeat("0", 64),
			"the instance secret file " + path + " is not 64 hexadecimal digits",
		},
	} {
		// Serial subtests corrupt and reread the same instance-secret fixture.
		t.Run(scenario.name, func(t *testing.T) {
			if err := os.WriteFile(path, []byte(scenario.value), 0o600); err != nil {
				t.Fatal(err)
			}
			_, err := loadSecret(t.Context(), directory)
			if err == nil || err.Error() != scenario.want {
				t.Fatalf("wrong secret error: %v", err)
			}
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			if string(data) != scenario.value {
				t.Fatal("corrupt secret was repaired")
			}
		})
	}
	if err := os.WriteFile(path, []byte(strings.Repeat("AB", 32)+"\n\u2003"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadSecret(t.Context(), directory); err != nil {
		t.Fatalf("trailing Unicode whitespace refused: %v", err)
	}
}
