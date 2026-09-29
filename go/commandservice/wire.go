package commandservice

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"reflect"
	"regexp"

	"github.com/wspl/demi/go/internal/wire"
)

// The wire's types are declared beside their rules: cmd/wiregen reads the
// declarations marked //demi:wire, //demi:union and //demi:variant, and writes
// the *_wire.go files, which hold each type's decoder, its rule check and the
// package's json options
// (docs/internal/go-migration/design/wire-contracts.md). Run `go generate`
// after changing a declaration.

// An InvalidError means a value from outside the process, or one about to
// leave it, breaks the rules of its wire type. It names the field and the rule,
// never the value: a value may be a secret, and errors are logged.
type InvalidError = wire.InvalidError

// Decode checks data, one JSON document from outside the process, and returns
// it as a T, one of the wire's types. It refuses a document over
// [MaxMetadataBytes] with [ErrTooLarge], and one that is not valid JSON, breaks
// the structure of T (a required member missing, an unknown member, a null
// where a value is optional, a value of another JSON kind) or breaks a rule of
// a field with an [*InvalidError], or with several of them joined when the
// rules of the fields are broken. The error names the field and the rule, never
// the value: a value may be a secret.
func Decode[T any](data []byte) (T, error) {
	var value T
	if len(data) > MaxMetadataBytes {
		return value, ErrTooLarge
	}
	if err := json.Unmarshal(data, &value, wireOptions); err != nil {
		return value, wire.Refusal(err)
	}
	if err := check(value); err != nil {
		return value, err
	}
	return value, nil
}

// Encode returns the JSON of value, one of the wire's types, after checking it
// as [Decode] would, so the SDK never sends what its peer would refuse.
func Encode[T any](value T) ([]byte, error) {
	if err := check(value); err != nil {
		return nil, err
	}
	data, err := json.Marshal(value, wireOptions)
	if err != nil {
		return nil, wire.Refusal(err)
	}
	if len(data) > MaxMetadataBytes {
		return nil, ErrTooLarge
	}
	return data, nil
}

// check runs the generated rule check of a wire type. A value that is absent
// (a nil interface, or a nil pointer) is refused as required.
func check(value any) error {
	if value == nil {
		return wire.Required("")
	}
	// A nil pointer to a wire type has methods, and calling them would panic.
	if pointer := reflect.ValueOf(value); pointer.Kind() == reflect.Pointer && pointer.IsNil() {
		return wire.Required("")
	}
	checked, ok := value.(interface{ validate() error })
	if !ok {
		return fmt.Errorf("commandservice: %T is not a type of the wire", value)
	}
	return checked.validate()
}

// The patterns of the rules of the wire's fields.
var (
	// nameCharacters are the characters of a conversation's name.
	nameCharacters = regexp.MustCompile(`^[A-Za-z0-9_-]+$`)
	// envName is the name of an environment variable: it holds no NUL and no
	// equals sign.
	envName = regexp.MustCompile(`^[^\x00=]+$`)
	// packageID is the id of a native command package.
	packageID = regexp.MustCompile(`^[a-z0-9]+(?:[.-][a-z0-9]+)+$`)
	// sha256Hex is a SHA-256 in lower-case hexadecimal.
	sha256Hex = regexp.MustCompile(`^[0-9a-f]{64}$`)
)

// requireObject is the rule of a field that holds JSON that its owner checks:
// the wire only knows it is an object.
func requireObject(value jsontext.Value) error {
	if value.Kind() != '{' {
		return errors.New("must be an object")
	}
	return nil
}

// absolutePath is the rule of a path that the runner and the command both
// resolve.
func absolutePath(path string) error {
	if !filepath.IsAbs(path) {
		return errors.New("is not an absolute path")
	}
	return nil
}

// downloadURL refuses a URL a runner must not download from: an HTTP or HTTPS
// URL without credentials is one it may.
func downloadURL(raw string) error {
	address, err := url.Parse(raw)
	if err != nil {
		// url.Parse quotes the part of the URL it could not read.
		return errors.New("is not a valid URL")
	}
	web := address.Scheme == "http" || address.Scheme == "https"
	credentials := false
	if address.User != nil {
		_, hasPassword := address.User.Password()
		credentials = address.User.Username() != "" || hasPassword
	}
	if !web || address.Host == "" || credentials {
		return errors.New("must be an HTTP or HTTPS URL without credentials")
	}
	return nil
}
