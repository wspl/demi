package backend

import (
	"bytes"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/providers"
)

// TestInstanceSecretPersistenceAndKeys
// checks secret persistence, corruption refusal and key derivation.
// Local files only; checks first-use publication, restart identity, corruption
// refusal and independent key labels without starting the backend or a vendor.
func TestInstanceSecretPersistenceAndKeys(t *testing.T) {
	t.Parallel()
	directory := t.TempDir()
	first, err := loadSecret(t.Context(), directory)
	if err != nil {
		t.Fatal(err)
	}
	again, err := loadSecret(t.Context(), directory)
	if err != nil {
		t.Fatal(err)
	}
	if first != again {
		t.Fatal("restart changed the instance secret")
	}
	path := filepath.Join(directory, "instance-secret")
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" && info.Mode().Perm() != 0o600 {
		t.Fatalf("secret permissions: %o", info.Mode().Perm())
	}
	secret, err := ParseInstanceSecret("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
	if err != nil {
		t.Fatal(err)
	}
	keys, err := secret.serviceKeys()
	if err != nil {
		t.Fatal(err)
	}
	// Independent HMAC-SHA256 extract/expand vectors, with a zero salt and
	// the Rust labels (one block, suffixed with byte 1).
	if hex.EncodeToString(keys.EmailCodes[:]) != "d02e61f8c078484c43cdfbf10bb81f1742a335a7871526b189ce947e202103ca" {
		t.Fatal("email key differs from HKDF vector")
	}
	vaultBytes, err := hex.DecodeString("5a76cb6d56804a2c62f3b8f8558930481d501c87730c7bbe7966db31c6057d43")
	if err != nil {
		t.Fatal(err)
	}
	expected := providers.NewVaultKey([32]byte(vaultBytes))
	row := providers.ConfigRow{Provider: "provider-1"}
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
		kind        SecretErrorKind
	}{
		{"malformed", "not-a-secret", SecretCorrupt},
		{"invalid UTF8", string([]byte{0xff}), SecretRead},
		{"leading space", " " + strings.Repeat("0", 64), SecretCorrupt},
	} {
		// Serial subtests corrupt and reread the same instance-secret fixture.
		t.Run(scenario.name, func(t *testing.T) {
			if err := os.WriteFile(path, []byte(scenario.value), 0o600); err != nil {
				t.Fatal(err)
			}
			_, err := loadSecret(t.Context(), directory)
			var failure *SecretError
			if !errors.As(err, &failure) || failure.Kind != scenario.kind {
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
		t.Fatalf("Rust trim_end whitespace: %v", err)
	}
}

// TestCLIExplicitEmptyValues
// checks the difference between absent and explicitly empty CLI values.
// Parsing alone costs no IO and verifies absence never swallows explicit emptiness.
func TestCLIExplicitEmptyValues(t *testing.T) {
	t.Parallel()
	required := []string{
		"--mode=shared",
		"--public-url=http://localhost:3271",
		"--machines-socket=",
		"--native-config=",
	}
	c, err := ParseConfig(required, nil)
	if err != nil {
		t.Fatal(err)
	}
	if c.Data != nil || c.ObjectStoreConfig != nil || c.WebDirectory != nil || c.RunnerReleaseDir != nil {
		t.Fatal("absent paths acquired values")
	}
	for _, flag := range []string{"data", "object-store-config", "web-directory", "runner-release-dir"} {
		args := append(append([]string{}, required...), "--"+flag+"=")
		c, err := ParseConfig(args, nil)
		if err != nil {
			t.Fatal(err)
		}
		values := map[string]*string{
			"data":                c.Data,
			"object-store-config": c.ObjectStoreConfig,
			"web-directory":       c.WebDirectory,
			"runner-release-dir":  c.RunnerReleaseDir,
		}
		if values[flag] == nil || *values[flag] != "" {
			t.Fatalf("explicit empty --%s was lost", flag)
		}
	}
	for _, env := range []string{
		"DEMI_INSTANCE_SECRET=",
		"DEMI_BACKEND_PUBLIC_URL=",
		"DEMI_EXPOSE_DOMAIN=",
		"DEMI_BACKEND_PORT=",
		"DEMI_CLAUDE_RELEASES_URL=",
	} {
		args := []string{"--mode=shared", "--machines-socket=", "--native-config="}
		if !strings.HasPrefix(env, "DEMI_BACKEND_PUBLIC_URL=") {
			args = append(args, "--public-url=http://localhost:3271")
		}
		if _, err := ParseConfig(args, []string{env}); err == nil {
			t.Fatalf("accepted unusable explicit value %s", env)
		}
	}
}
