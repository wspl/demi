// Package auth implements the backend's account authentication policies.
package auth

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"fmt"
	"runtime"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/webapi"
	"golang.org/x/crypto/argon2"
)

type PasswordHasher struct {
	permits chan struct{}
	dummy   storage.PasswordHash
}

func NewPasswordHasher(ctx context.Context) (*PasswordHasher, error) {
	h := &PasswordHasher{permits: make(chan struct{}, max(1, runtime.GOMAXPROCS(0)))}
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		return nil, err
	}
	dummy, err := h.Hash(ctx, webapi.Password(base64.RawStdEncoding.EncodeToString(secret)))
	if err != nil {
		return nil, err
	}
	h.dummy = dummy
	return h, nil
}
func (h *PasswordHasher) acquire(ctx context.Context) error {
	select {
	case h.permits <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
func (h *PasswordHasher) Hash(ctx context.Context, password webapi.Password) (storage.PasswordHash, error) {
	if err := h.acquire(ctx); err != nil {
		return storage.PasswordHash{}, err
	}
	defer func() { <-h.permits }()
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return storage.PasswordHash{}, err
	}
	hash := argon2.IDKey([]byte(password.Expose()), salt, 2, 19456, 1, 32)
	return storage.ParsePasswordHash("$argon2id$v=19$m=19456,t=2,p=1$" + base64.RawStdEncoding.EncodeToString(salt) + "$" + base64.RawStdEncoding.EncodeToString(hash))
}
func (h *PasswordHasher) Verify(ctx context.Context, password webapi.Password, stored *storage.PasswordHash) (bool, error) {
	if err := h.acquire(ctx); err != nil {
		return false, err
	}
	defer func() { <-h.permits }()
	against := h.dummy
	if stored != nil {
		against = *stored
	}
	matches, err := verify(password, against)
	return stored != nil && matches, err
}
func verify(password webapi.Password, stored storage.PasswordHash) (bool, error) {
	parts := strings.Split(stored.PHC(), "$")
	if len(parts) == 5 {
		parts = append(parts[:2], append([]string{"v=19"}, parts[2:]...)...)
	}
	if len(parts) != 6 {
		return false, fmt.Errorf("argon2 failed: password hash string missing field")
	}
	if parts[1] != "argon2id" && parts[1] != "argon2i" {
		return false, fmt.Errorf("argon2 failed: unsupported algorithm")
	}
	if parts[2] != "v=19" {
		return false, fmt.Errorf("argon2 failed: unsupported version")
	}
	params := map[string]uint32{"m": 19456, "t": 2, "p": 1}
	for field := range strings.SplitSeq(parts[3], ",") {
		if field == "" {
			continue
		}
		key, value, ok := strings.Cut(field, "=")
		n, err := strconv.ParseUint(value, 10, 32)
		if !ok || err != nil || (key != "m" && key != "t" && key != "p") {
			return false, fmt.Errorf("argon2 failed: invalid parameter")
		}
		params[key] = uint32(n)
	}
	memory, rounds, lanes := params["m"], params["t"], params["p"]
	if lanes < 1 || lanes > 255 || rounds < 1 || memory < 8*lanes {
		return false, fmt.Errorf("argon2 failed: invalid parameter")
	}
	salt, err := base64.RawStdEncoding.Strict().DecodeString(parts[4])
	if err != nil || len(salt) < 8 {
		return false, fmt.Errorf("argon2 failed: salt invalid")
	}
	expected, err := base64.RawStdEncoding.Strict().DecodeString(parts[5])
	if err != nil || len(expected) < 4 {
		return false, fmt.Errorf("argon2 failed: output invalid")
	}
	derive := argon2.IDKey
	if parts[1] == "argon2i" {
		derive = argon2.Key
	}
	actual := derive([]byte(password.Expose()), salt, rounds, memory, uint8(lanes), uint32(len(expected)))
	return subtle.ConstantTimeCompare(actual, expected) == 1, nil
}
