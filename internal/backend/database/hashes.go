package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

// PasswordHash is a stored PHC password hash, including algorithm and parameters.
// Its zero value is invalid; ParsePasswordHash validates storage input.
type PasswordHash struct{}

// ParsePasswordHash reads a PHC string back from storage.
func ParsePasswordHash(text string) (PasswordHash, error) { panic("not written: b-database") }

// Text returns the PHC string for password verification and storage.
func (h PasswordHash) Text() string { panic("not written: b-database") }

// String redacts the password hash in diagnostics.
func (h PasswordHash) String() string { panic("not written: b-database") }

// TokenHash is the lowercase hexadecimal SHA-256 of a session or device token.
type TokenHash struct{}

// HashToken hashes a token for storage and lookup.
func HashToken(token string) TokenHash { panic("not written: b-database") }

// Text returns the stored token hash.
func (h TokenHash) Text() string { panic("not written: b-database") }

// CodeHash is the HMAC-SHA256 of a challenge's id and code under the email-change key.
type CodeHash struct{}

// HashCode hashes code for challenge under the email-change key.
func HashCode(key []byte, challenge, code string) CodeHash { panic("not written: b-database") }

// Text returns the stored challenge hash.
func (h CodeHash) Text() string { panic("not written: b-database") }

// Matches compares in constant time without revealing a guess's matching prefix.
func (h CodeHash) Matches(stored string) bool { panic("not written: b-database") }
