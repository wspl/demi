package storage

import (
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

// PasswordHash is a PHC string. Its formatting methods never reveal the hash.
type PasswordHash struct{ phc string }

var phcID = regexp.MustCompile(`^[a-z0-9-]{1,32}$`)
var phcValue = regexp.MustCompile(`^[A-Za-z0-9/+.-]+$`)

// ParsePasswordHash follows password-hash 0.5's PHC syntax, independently of
// whether an algorithm can be verified by this process.
func ParsePasswordHash(text string) (PasswordHash, error) {
	fail := func(reason string) (PasswordHash, error) { return PasswordHash{}, errors.New(reason) }
	parts := strings.Split(text, "$")
	if len(parts) < 2 || parts[0] != "" {
		return fail("password hash string missing field")
	}
	if !phcID.MatchString(parts[1]) {
		return fail("invalid parameter name")
	}
	i := 2
	if i < len(parts) && strings.HasPrefix(parts[i], "v=") && !strings.Contains(parts[i], ",") {
		value := strings.TrimPrefix(parts[i], "v=")
		if value == "" {
			return fail("invalid parameter value: value malformed")
		}
		for _, c := range value {
			if c < '0' || c > '9' {
				return fail(fmt.Sprintf("invalid parameter value: contains invalid character: '%c'", c))
			}
		}
		if len(value) > 1 && value[0] == '0' {
			return fail("invalid parameter value: value format is invalid")
		}
		if _, err := strconv.ParseUint(value, 10, 32); err != nil {
			return fail("invalid parameter value: value format is invalid")
		}
		i++
	}
	if i < len(parts) && strings.Contains(parts[i], "=") {
		if len(parts[i]) > 127 {
			return fail("maximum number of parameters reached")
		}
		seen := map[string]bool{}
		for item := range strings.SplitSeq(parts[i], ",") {
			key, value, ok := strings.Cut(item, "=")
			if !ok || !phcID.MatchString(key) {
				return fail("invalid parameter name")
			}
			if seen[key] {
				return fail("duplicate parameter")
			}
			if len(value) > 64 {
				return fail("invalid parameter value: value to long")
			}
			if value != "" && !phcValue.MatchString(value) {
				return fail("invalid parameter value: value malformed")
			}
			seen[key] = true
		}
		i++
	}
	if i < len(parts) {
		salt := parts[i]
		if len(salt) < 4 {
			return fail("salt invalid: value to short")
		}
		if len(salt) > 64 {
			return fail("salt invalid: value to long")
		}
		if !phcValue.MatchString(salt) {
			return fail("salt invalid: value malformed")
		}
		i++
	}
	if i < len(parts) {
		hash, err := base64.RawStdEncoding.Strict().DecodeString(parts[i])
		if err != nil {
			return fail("invalid Base64 encoding")
		}
		if len(hash) < 10 {
			return fail("output size too short, expected at least 10 bytes")
		}
		if len(hash) > 64 {
			return fail("output size too long, expected at most 64 bytes")
		}
		i++
	}
	if i < len(parts) {
		return fail("password hash string contains trailing characters")
	}
	return PasswordHash{text}, nil
}
func (h PasswordHash) PHC() string      { return h.phc }
func (h PasswordHash) String() string   { return "PasswordHash(..)" }
func (h PasswordHash) GoString() string { return h.String() }

type TokenHash struct{ text string }

func HashToken(token string) TokenHash {
	sum := sha256.Sum256([]byte(token))
	return TokenHash{hex.EncodeToString(sum[:])}
}
func (h TokenHash) Text() string   { return h.text }
func (h TokenHash) String() string { return "TokenHash(..)" }

type CodeHash struct{ text string }

func HashCode(key []byte, challenge, code string) CodeHash {
	mac := hmac.New(sha256.New, key)
	mac.Write([]byte("email-change:" + challenge + ":" + code))
	return CodeHash{hex.EncodeToString(mac.Sum(nil))}
}
func (h CodeHash) matches(text string) bool {
	return subtle.ConstantTimeCompare([]byte(h.text), []byte(text)) == 1
}
func (h CodeHash) String() string { return "CodeHash(..)" }
