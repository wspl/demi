package accounts

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"runtime"
	"strconv"
	"strings"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
	"golang.org/x/crypto/argon2"
	"golang.org/x/sync/semaphore"
)

// HashError is why hashing or verifying failed.
type HashError struct{ Err error }

func (e *HashError) Error() string { return "argon2 failed: " + e.Err.Error() }
func (e *HashError) Unwrap() error { return e.Err }

// PasswordHasher bounds concurrent password hashes to the runtime's available CPU parallelism.
// An admitted hash runs in its caller's goroutine and retains its permit until
// completion, even if the request is canceled while Argon2 runs.
type PasswordHasher struct {
	permits *semaphore.Weighted
	dummy   string
}

// NewPasswordHasher makes the fixed dummy hash used for unknown accounts.
func NewPasswordHasher(ctx context.Context) (*PasswordHasher, error) {
	return newPasswordHasher(ctx, runtime.GOMAXPROCS(0))
}

// newPasswordHasher initializes password hashing with the supplied admission limit.
func newPasswordHasher(ctx context.Context, permits int) (*PasswordHasher, error) {
	h := &PasswordHasher{permits: semaphore.NewWeighted(int64(permits))}
	secret, err := uuid.NewRandom()
	if err != nil {
		return nil, fmt.Errorf("generate dummy password: %w", err)
	}
	dummy, err := h.hash(ctx, webapi.Password(secret.String()))
	if err != nil {
		return nil, err
	}
	h.dummy = dummy
	return h, nil
}

// Hash hashes a password with Rust Argon2's default parameters and PHC spelling.
func (h *PasswordHasher) Hash(ctx context.Context, password webapi.Password) (database.PasswordHash, error) {
	text, err := h.hash(ctx, password)
	if err != nil {
		return database.PasswordHash{}, err
	}
	stored, err := database.ParsePasswordHash(text)
	if err != nil {
		return database.PasswordHash{}, &HashError{Err: err}
	}
	return stored, nil
}

func (h *PasswordHasher) hash(ctx context.Context, password webapi.Password) (string, error) {
	if err := h.permits.Acquire(ctx, 1); err != nil {
		return "", err
	}
	defer h.permits.Release(1)
	var salt [16]byte
	rand.Read(salt[:])
	return hashPassword(password, salt[:]), nil
}

// Verify checks a password against stored; nil spends the same work against a
// dummy hash and always answers false.
func (h *PasswordHasher) Verify(ctx context.Context, password webapi.Password, stored *database.PasswordHash) (bool, error) {
	against := h.dummy
	if stored != nil {
		against = stored.Text()
	}
	matches, err := h.verify(ctx, password, against)
	return stored != nil && matches, err
}

func (h *PasswordHasher) verify(ctx context.Context, password webapi.Password, stored string) (bool, error) {
	if err := h.permits.Acquire(ctx, 1); err != nil {
		return false, err
	}
	defer h.permits.Release(1)
	return verifyPassword(password, stored)
}

// hashPassword writes the PHC representation used by Rust's default Argon2.
func hashPassword(password webapi.Password, salt []byte) string {
	key := argon2.IDKey([]byte(password), salt, 2, 19456, 1, 32)
	return "$argon2id$v=19$m=19456,t=2,p=1$" + base64.RawStdEncoding.EncodeToString(salt) + "$" + base64.RawStdEncoding.EncodeToString(key)
}

// verifyPassword reads the algorithm and costs from a stored PHC string.
func verifyPassword(password webapi.Password, stored string) (bool, error) {
	parsed, err := parsePHC(stored)
	if err != nil {
		return false, &HashError{Err: err}
	}
	var key []byte
	switch parsed.algorithm {
	case "argon2id":
		key = argon2.IDKey([]byte(password), parsed.salt, parsed.iterations, parsed.memory, parsed.threads, uint32(len(parsed.hash)))
	case "argon2i":
		key = argon2.Key([]byte(password), parsed.salt, parsed.iterations, parsed.memory, parsed.threads, uint32(len(parsed.hash)))
	}
	return subtle.ConstantTimeCompare(key, parsed.hash) == 1, nil
}

type phc struct {
	algorithm          string
	memory, iterations uint32
	threads            uint8
	salt, hash         []byte
}

var errInvalidPHC = errors.New("invalid password hash")
var errUnsupportedPHC = errors.New("unsupported Argon2 parameters")

func parsePHC(text string) (phc, error) {
	p := phc{memory: 19456, iterations: 2, threads: 1}
	fields := strings.Split(text, "$")
	if len(fields) < 5 || fields[0] != "" {
		return p, errInvalidPHC
	}
	p.algorithm = fields[1]
	if p.algorithm != "argon2id" && p.algorithm != "argon2i" {
		return p, errUnsupportedPHC
	}
	fields = fields[2:]
	if strings.HasPrefix(fields[0], "v=") {
		if fields[0] != "v=19" {
			return p, errUnsupportedPHC
		}
		fields = fields[1:]
	}
	if len(fields) != 3 {
		return p, errInvalidPHC
	}
	seen := make(map[string]bool)
	for _, param := range strings.Split(fields[0], ",") {
		name, value, ok := strings.Cut(param, "=")
		if !ok || seen[name] || value == "" || (len(value) > 1 && value[0] == '0') {
			return p, errInvalidPHC
		}
		seen[name] = true
		for _, c := range value {
			if c < '0' || c > '9' {
				return p, errInvalidPHC
			}
		}
		n, err := strconv.ParseUint(value, 10, 32)
		if err != nil {
			return p, fmt.Errorf("%w: %w", errInvalidPHC, err)
		}
		switch name {
		case "m":
			p.memory = uint32(n)
		case "t":
			p.iterations = uint32(n)
		case "p":
			if n > 255 {
				return p, errUnsupportedPHC
			}
			p.threads = uint8(n)
		default:
			return p, errUnsupportedPHC
		}
	}
	if p.threads == 0 || p.iterations == 0 || p.memory < 8*uint32(p.threads) {
		return p, errInvalidPHC
	}
	var err error
	p.salt, err = base64.RawStdEncoding.Strict().DecodeString(fields[1])
	if err != nil {
		return p, fmt.Errorf("%w: %w", errInvalidPHC, err)
	}
	if len(p.salt) < 8 || len(fields[1]) > 64 {
		return p, errInvalidPHC
	}
	p.hash, err = base64.RawStdEncoding.Strict().DecodeString(fields[2])
	if err != nil {
		return p, fmt.Errorf("%w: %w", errInvalidPHC, err)
	}
	if len(p.hash) < 10 || len(p.hash) > 64 {
		return p, errInvalidPHC
	}
	return p, nil
}

// String keeps the dummy password hash out of diagnostics.
func (*PasswordHasher) String() string { return "PasswordHasher(..)" }
