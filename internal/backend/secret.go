package backend

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

// InstanceSecret is the instance secret. Formatting never shows its bytes.
type InstanceSecret struct{}

// ParseInstanceSecret reads exactly 64 hexadecimal digits. Its error never
// includes the supplied text.
func ParseInstanceSecret(text string) (InstanceSecret, error) { panic("not written: b-backend") }

// String hides the secret in ordinary formatting.
func (s InstanceSecret) String() string { panic("not written: b-backend") }

// GoString hides the secret in Go-syntax formatting.
func (s InstanceSecret) GoString() string { panic("not written: b-backend") }

// SecretErrorKind identifies why the instance secret is unusable.
type SecretErrorKind uint8

const (
	// SecretMalformed means the configured secret is not 64 hexadecimal digits.
	SecretMalformed SecretErrorKind = iota
	// SecretCorrupt means the persisted secret is not 64 hexadecimal digits.
	SecretCorrupt
	// SecretRead means the persisted secret could not be read.
	SecretRead
	// SecretCreate means the first-use secret could not be published.
	SecretCreate
)

// SecretError is why the instance secret is unusable. A malformed secret
// stops startup, and no error shows the secret itself.
type SecretError struct {
	Kind SecretErrorKind
	Path string
	Err  error
}

// Error describes the failure without revealing the secret.
func (e *SecretError) Error() string { panic("not written: b-backend") }

// Unwrap returns the underlying read or publication error.
func (e *SecretError) Unwrap() error { panic("not written: b-backend") }
