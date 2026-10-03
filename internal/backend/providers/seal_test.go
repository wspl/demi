package providers

import (
	"bytes"
	"encoding/hex"
	"errors"
	"os"
	"strings"
	"testing"

	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// This test pins ciphertext written by the unchanged Rust implementation, as
// well as row/key binding and fresh nonces. It needs no external resources.
func TestSealedValueOpensOnlyForItsRowUnderItsKey(t *testing.T) {
	var keyBytes [32]byte
	for i := range keyBytes {
		keyBytes[i] = 7
	}
	key := NewVaultKey(keyBytes)
	fixture, err := os.ReadFile("testdata/rust-sealed.txt")
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(strings.TrimSpace(string(fixture)), "\n") {
		kind, text, _ := strings.Cut(line, " ")
		sealed, err := hex.DecodeString(text)
		if err != nil {
			t.Fatal(err)
		}
		var row Row = ConfigRow{Provider: "entry-1"}
		want := `{"apiKey":"sk-test-123"}`
		if kind == "secret" {
			row = SecretRow{Provider: "entry-1", Account: "cred-1"}
			want = `{"refresh":"rust-token"}`
		}
		plain, err := key.Open(row, sealed)
		if err != nil || string(plain) != want {
			t.Fatalf("Rust %s record: %q, %v", kind, plain, err)
		}
	}
	row := ConfigRow{Provider: "entry-1"}
	sealed, err := key.Seal(row, []byte(`{"apiKey":"sk-test-123"}`))
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(sealed, []byte("sk-test-123")) {
		t.Fatal("plaintext in sealed row")
	}
	plain, err := key.Open(row, sealed)
	if err != nil || string(plain) != `{"apiKey":"sk-test-123"}` {
		t.Fatalf("Go round trip: %q, %v", plain, err)
	}
	first, err := key.Seal(row, []byte("same"))
	if err != nil {
		t.Fatal(err)
	}
	second, err := key.Seal(row, []byte("same"))
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(first, second) {
		t.Fatal("nonce reused")
	}
	otherKey := keyBytes
	otherKey[0] = 8
	tampered := bytes.Clone(sealed)
	tampered[len(tampered)-1] ^= 1
	cases := []struct {
		name  string
		key   *VaultKey
		row   Row
		bytes []byte
	}{
		{
			"entry",
			key,
			ConfigRow{Provider: "entry-2"},
			sealed,
		},
		{
			"kind",
			key,
			SecretRow{Provider: "entry-1", Account: "cred-1"},
			sealed,
		},
		{
			"key",
			NewVaultKey(otherKey),
			row,
			sealed,
		},
		{
			"altered",
			key,
			row,
			tampered,
		},
		{
			"short",
			key,
			row,
			sealed[:8],
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := c.key.Open(c.row, c.bytes)
			var bad *Unsealable
			if !errors.As(err, &bad) {
				t.Fatalf("wanted authentication failure, got %v", err)
			}
		})
	}
	sealed, err = key.Seal(SecretRow{Provider: "ab", Account: "c"}, []byte("x"))
	if err != nil {
		t.Fatal(err)
	}
	var bad *Unsealable
	if _, err = key.Open(SecretRow{Provider: "a", Account: "bc"}, sealed); !errors.As(err, &bad) {
		t.Fatal("ambiguous row identity")
	}
}
