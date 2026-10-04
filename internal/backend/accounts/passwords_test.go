package accounts

import (
	"context"
	"encoding/base64"
	"errors"
	"os"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// The fixtures come from the reference writer described in testdata/README.md.
// The test verifies each stored hash and checks that hashPassword writes it byte for byte.
func TestPasswordFixtures(t *testing.T) {
	h, err := NewPasswordHasher(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	control := databasetest.Control(t.Context(), t, core.SystemClock{})
	user := databasetest.Master(t.Context(), t, control)
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
			t.Fatal("hash differs from the fixture's PHC string")
		}
		hash, err := database.ParsePasswordHash(stored)
		if err != nil {
			t.Fatal(err)
		}
		if err := control.SetPassword(t.Context(), user.ID, hash); err != nil {
			t.Fatal(err)
		}
		account, found, err := control.Account(t.Context(), user.ID)
		if err != nil || !found {
			t.Fatalf("stored account: %v", err)
		}
		valid, err := h.Verify(t.Context(), password, &account.PasswordHash)
		if err != nil || !valid {
			t.Fatalf("fixture password rejected: %v", err)
		}
		valid, err = h.Verify(t.Context(), password+"wrong", &account.PasswordHash)
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
		if err == nil || !strings.HasPrefix(err.Error(), "argon2 failed: ") {
			t.Fatalf("expected an argon2 failure for %q, got %v", stored, err)
		}
	}
}

func TestHashAdmissionCancellationReleasesCapacity(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		h, err := newPasswordHasher(t.Context(), 1)
		if err != nil {
			t.Fatal(err)
		}
		func() {
			if err := h.permits.Acquire(t.Context(), 1); err != nil {
				t.Fatal(err)
			}
			defer h.permits.Release(1)
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			done := make(chan error, 1)
			go func() {
				_, err := h.Hash(ctx, "password")
				done <- err
			}()
			synctest.Wait()
			cancel()
			if err := <-done; !errors.Is(err, context.Canceled) {
				t.Fatalf("waiting hash: %v", err)
			}
		}()
		if !h.permits.TryAcquire(1) {
			t.Fatal("canceled waiter leaked a permit")
		}
		defer h.permits.Release(1)
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
	if err == nil || !strings.HasPrefix(err.Error(), "argon2 failed: ") {
		t.Fatalf("dummy was not verified: %v", err)
	}
}

func TestPasswordWriterMatchesVerifiedFixture(t *testing.T) {
	data, err := os.ReadFile("testdata/go-password.tsv")
	if err != nil {
		t.Fatal(err)
	}
	fields := strings.Split(strings.TrimSuffix(string(data), "\n"), "\t")
	if len(fields) != 3 {
		t.Fatal("invalid Go fixture")
	}
	if got := hashPassword(webapi.Password(fields[0]), []byte(fields[1])); got != fields[2] {
		t.Fatal("hashPassword output differs from the verified fixture")
	}
}
