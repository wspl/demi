package accounts

import (
	"context"
	"encoding/base64"
	"errors"
	"os"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/webapi"
	"golang.org/x/sync/semaphore"
)

// These fixtures are written by testdata/reference with Rust argon2 0.5.3.
// The test covers both reading Rust hashes and byte-identical Go output.
func TestRustPasswordFixtures(t *testing.T) {
	data, err := os.ReadFile("testdata/passwords.tsv")
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range strings.Split(strings.TrimSuffix(string(data), "\n"), "\n") {
		fields := strings.Split(row, "\t")
		if len(fields) != 3 {
			t.Fatal("invalid fixture row")
		}
		salt, err := base64.RawStdEncoding.DecodeString(fields[1])
		if err != nil {
			t.Fatal(err)
		}
		password := webapi.Password(fields[0])
		stored := fields[2]
		if got := hashPassword(password, salt); got != stored {
			t.Fatal("Go hash differs from Rust PHC")
		}
		valid, err := verifyPassword(password, stored)
		if err != nil || !valid {
			t.Fatalf("Rust password rejected: %v", err)
		}
		valid, err = verifyPassword(password+"wrong", stored)
		if err != nil || valid {
			t.Fatalf("wrong password accepted or errored: %v", err)
		}
	}
}

func TestMalformedHashesReturnErrors(t *testing.T) {
	cases := []string{
		"", "$argon2id", "$argon2id$v=19$m=8,t=0,p=1$c29tZXNhbHQ$MTIzNDU2Nzg5MDEyMzQ1Ng",
		"$argon2id$v=19$m=8,t=1,p=0$c29tZXNhbHQ$MTIzNDU2Nzg5MDEyMzQ1Ng",
		"$argon2id$v=19$m=8,t=1,p=2$c29tZXNhbHQ$MTIzNDU2Nzg5MDEyMzQ1Ng",
		"$argon2id$v=19$m=8,t=1,p=1,p=1$c29tZXNhbHQ$MTIzNDU2Nzg5MDEyMzQ1Ng",
		"$argon2id$v=19$m=8,t=1,p=1$!$MTIzNDU2Nzg5MDEyMzQ1Ng",
	}
	for _, stored := range cases {
		_, err := verifyPassword("password", stored)
		var hashErr *HashError
		if !errors.As(err, &hashErr) {
			t.Fatalf("expected HashError for %q, got %v", stored, err)
		}
	}
}

func TestHashAdmissionCancellationReleasesCapacity(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		h := &PasswordHasher{permits: semaphore.NewWeighted(1)}
		if err := h.permits.Acquire(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		go func() { _, err := h.hash(ctx, "password"); done <- err }()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("waiting hash: %v", err)
		}
		h.permits.Release(1)
		if !h.permits.TryAcquire(1) {
			t.Fatal("canceled waiter leaked a permit")
		}
		h.permits.Release(1)
	})
}

func TestUnknownAccountRunsDummyVerification(t *testing.T) {
	h, err := NewPasswordHasher(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	matched, err := h.Verify(t.Context(), "anything", nil)
	if err != nil || matched {
		t.Fatalf("unknown account: %v %v", matched, err)
	}
	// A broken dummy reaches the parser, proving nil did not short-circuit.
	h.dummy = "broken"
	_, err = h.Verify(t.Context(), "anything", nil)
	var hashErr *HashError
	if !errors.As(err, &hashErr) {
		t.Fatalf("dummy was not verified: %v", err)
	}
}

func TestGoPasswordFixtureVerifiedByRust(t *testing.T) {
	data, err := os.ReadFile("testdata/go-password.tsv")
	if err != nil {
		t.Fatal(err)
	}
	fields := strings.Split(strings.TrimSuffix(string(data), "\n"), "\t")
	if len(fields) != 3 {
		t.Fatal("invalid Go fixture")
	}
	if got := hashPassword(webapi.Password(fields[0]), []byte(fields[1])); got != fields[2] {
		t.Fatal("Go writer changed from the fixture verified by Rust")
	}
}
