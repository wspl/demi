package auth

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

// Cost: three 19 MiB Argon2 verifications; the fixture was emitted by Rust.
func TestRustPHCAuthenticatesOnlyItsPassword(t *testing.T) {
	h, err := NewPasswordHasher(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile("../storage/testdata/rust/control.sqlite")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "control.sqlite")
	if err = os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	c, err := storage.OpenControl(t.Context(), path, core.SystemClock{})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	id, _ := webapi.ParseUserID("rust-user")
	account, err := c.Account(t.Context(), id)
	if err != nil || account == nil {
		t.Fatalf("Rust account: %v", err)
	}
	hash := account.Password

	for _, test := range []struct {
		password string
		want     bool
	}{{"fixture password", true}, {"other password", false}} {
		ok, err := h.Verify(t.Context(), webapi.Password(test.password), &hash)
		if err != nil || ok != test.want {
			t.Fatalf("Rust hash: matched=%v error=%v", ok, err)
		}
	}
}
