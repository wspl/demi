package database

import (
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"
)

// PasswordHash is a stored PHC password hash, including algorithm and parameters.
// Its zero value is invalid; ParsePasswordHash validates storage input.
type PasswordHash struct{ value string }

// ParsePasswordHash reads a PHC string back from storage.
func ParsePasswordHash(text string) (PasswordHash, error) {
	// PHC syntax is checked here without selecting a password algorithm.
	parts := strings.Split(text, "$")
	if len(parts) < 2 || len(parts) > 6 || parts[0] != "" {
		return PasswordHash{}, fmt.Errorf("invalid password hash")
	}
	identifier := func(s string) bool {
		if len(s) == 0 || len(s) > 32 {
			return false
		}
		for _, c := range s {
			if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '-' {
				return false
			}
		}
		return true
	}
	if !identifier(parts[1]) {
		return PasswordHash{}, fmt.Errorf("invalid algorithm")
	}
	i := 2
	if i < len(parts) && strings.HasPrefix(parts[i], "v=") && !strings.Contains(parts[i], ",") {
		if len(parts[i][2:]) > 1 && parts[i][2] == '0' {
			return PasswordHash{}, fmt.Errorf("invalid version")
		}
		if _, err := strconv.ParseUint(parts[i][2:], 10, 32); err != nil {
			return PasswordHash{}, fmt.Errorf("invalid version: %w", err)
		}
		i++
	}
	if i < len(parts) && strings.Contains(parts[i], "=") {
		if len(parts[i]) > 127 {
			return PasswordHash{}, fmt.Errorf("invalid parameters")
		}
		for _, param := range strings.Split(parts[i], ",") {
			key, value, ok := strings.Cut(param, "=")
			if !ok || !identifier(key) || len(value) > 64 {
				return PasswordHash{}, fmt.Errorf("invalid parameters")
			}
			for _, c := range value {
				if (c < 'A' || c > 'Z') && (c < 'a' || c > 'z') && (c < '0' || c > '9') && !strings.ContainsRune("/+.-", c) {
					return PasswordHash{}, fmt.Errorf("invalid parameter value")
				}
			}
		}
		i++
	}
	if i < len(parts) {
		salt := parts[i]
		if len(salt) < 4 || len(salt) > 64 {
			return PasswordHash{}, fmt.Errorf("invalid salt length")
		}
		for _, c := range salt {
			if (c < 'A' || c > 'Z') && (c < 'a' || c > 'z') && (c < '0' || c > '9') && !strings.ContainsRune("/+.-", c) {
				return PasswordHash{}, fmt.Errorf("invalid salt")
			}
		}
		i++
	}
	if i < len(parts) {
		if strings.ContainsAny(parts[i], "\r\n") {
			return PasswordHash{}, fmt.Errorf("invalid hash")
		}
		hash, err := base64.RawStdEncoding.Strict().DecodeString(parts[i])
		if err != nil {
			return PasswordHash{}, fmt.Errorf("invalid hash: %w", err)
		}
		if len(hash) < 10 || len(hash) > 64 {
			return PasswordHash{}, fmt.Errorf("invalid hash length")
		}
		i++
	}
	if i != len(parts) {
		return PasswordHash{}, fmt.Errorf("invalid password hash")
	}
	return PasswordHash{value: text}, nil
}

// Text returns the PHC string for password verification and storage.
func (h PasswordHash) Text() string { return h.value }

// String redacts the password hash in diagnostics.
func (h PasswordHash) String() string {
	return "PasswordHash(..)"
}

// TokenHash is the lowercase hexadecimal SHA-256 of a session or device token.
type TokenHash struct{ value string }

// HashToken hashes a token for storage and lookup.
func HashToken(token string) TokenHash {
	digest := sha256.Sum256([]byte(token))
	return TokenHash{value: hex.EncodeToString(digest[:])}
}

// Text returns the stored token hash.
func (h TokenHash) Text() string { return h.value }

// CodeHash is the HMAC-SHA256 of a challenge's id and code under the email-change key.
type CodeHash struct{ value string }

// HashCode hashes code for challenge under the email-change key.
func HashCode(key []byte, challenge, code string) CodeHash {
	mac := hmac.New(sha256.New, key)
	_, _ = mac.Write([]byte("email-change:" + challenge + ":" + code))
	return CodeHash{value: hex.EncodeToString(mac.Sum(nil))}
}

// Text returns the stored challenge hash.
func (h CodeHash) Text() string { return h.value }

// Matches compares in constant time without revealing a guess's matching prefix.
func (h CodeHash) Matches(stored string) bool {
	return subtle.ConstantTimeCompare([]byte(h.value), []byte(stored)) == 1
}
