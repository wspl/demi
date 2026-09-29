// Package claudeproto is the contract of the demi.claude package
// (docs/providers/claude-code.md § The package): claude.ensure installs the
// Claude Code CLI a release record names and claude.status lists the
// installations. Each writes one JSON document to standard output, a failure
// included, because a service stream carries only standard output. The backend
// and the package decode these records with the same types.
package claudeproto

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/v2"
	"errors"
	"fmt"
	"net/url"
	"reflect"
	"regexp"
	"strconv"

	"github.com/wspl/demi/go/internal/wire"
)

// Package is the package's id in the native catalog.
const Package = "demi.claude"

// The package's operations.
const (
	// OperationEnsure installs the version a release record names.
	OperationEnsure = "claude.ensure"
	// OperationStatus lists the installed versions.
	OperationStatus = "claude.status"
)

// Operations returns the package's operations, in the order the descriptor lists
// them.
func Operations() []string {
	return []string{OperationEnsure, OperationStatus}
}

// An InvalidError means a record breaks the rules of its wire type. It names the
// field and the rule, never the value.
type InvalidError = wire.InvalidError

// sha256Hex is a SHA-256 in lowercase hex.
var sha256Hex = regexp.MustCompile(`^[a-f0-9]{64}$`)

// A Release is what claude.ensure reads from its input: one CLI version and its
// official executable for each platform key. Every entry is checked, not only
// this machine's.
//
//demi:wire
type Release struct {
	Version   string              `json:"version" check:"func=cliVersion"`
	Platforms map[string]Artifact `json:"platforms"`
}

// cliVersion is the rule of a version that names an installation directory.
func cliVersion(version string) error {
	if !IsVersion(version) {
		return errors.New("is not a SemVer version without build metadata")
	}
	return nil
}

// An Artifact says where one platform's executable is, its byte size and its
// SHA-256 in lowercase hex.
//
//demi:wire
type Artifact struct {
	URL    string `json:"url" check:"func=absoluteURL"`
	Size   uint64 `json:"size" check:"range=1.."`
	SHA256 string `json:"sha256" check:"pattern=sha256Hex"`
}

// absoluteURL is the rule of a URL: it parses and names its scheme, and an http
// or https URL names a host and a port that is at most 65535.
func absoluteURL(raw string) error {
	address, err := url.Parse(raw)
	if err != nil || address.Scheme == "" {
		return errors.New("is not a URL")
	}
	if address.Scheme != "http" && address.Scheme != "https" {
		return nil
	}
	if address.Hostname() == "" {
		return errors.New("is not a URL")
	}
	if port := address.Port(); port != "" {
		// A port is 16 bits.
		if _, err := strconv.ParseUint(port, 10, 16); err != nil {
			return errors.New("is not a URL")
		}
	}
	return nil
}

// An Installed is one usable executable.
//
//demi:wire
type Installed struct {
	Version string `json:"version"`
	Path    string `json:"path"`
}

// A Status is what claude.status answers: this machine's platform key and its
// installations, newest version first.
//
//demi:wire
type Status struct {
	Platform  string      `json:"platform"`
	Installed []Installed `json:"installed"`
}

// A Receipt is what a version directory records about the executable beside it.
//
//demi:wire
type Receipt struct {
	Version  string `json:"version"`
	Platform string `json:"platform"`
	SHA256   string `json:"sha256"`
	Size     uint64 `json:"size"`
}

// An ErrorCode says why an operation failed; a cancelled invocation writes no
// document.
type ErrorCode string

// The codes a caller branches on.
const (
	InvalidRelease      ErrorCode = "invalid_release"
	UnsupportedPlatform ErrorCode = "unsupported_platform"
	DownloadFailed      ErrorCode = "download_failed"
	VerificationFailed  ErrorCode = "verification_failed"
	InstallFailed       ErrorCode = "install_failed"
)

// A Reply is the document an operation writes: `{"ok": true, ...answer}`, or
// `{"ok": false, "code", "message"}`. The variants are told apart by their
// members.
//
//demi:union untagged
type Reply interface {
	reply()
}

// Ensured is the reply of claude.ensure that installed: the executable.
//
//demi:variant
type Ensured struct {
	OK      bool   `json:"ok" check:"func=isTrue"`
	Version string `json:"version"`
	Path    string `json:"path"`
}

// Listed is the reply of claude.status.
//
//demi:variant
type Listed struct {
	OK        bool        `json:"ok" check:"func=isTrue"`
	Platform  string      `json:"platform"`
	Installed []Installed `json:"installed"`
}

// Failed is the reply of an operation that failed.
//
//demi:variant
type Failed struct {
	OK      bool      `json:"ok" check:"func=isFalse"`
	Code    ErrorCode `json:"code" check:"oneof=invalid_release|unsupported_platform|download_failed|verification_failed|install_failed"`
	Message string    `json:"message"`
}

func (Ensured) reply() {}
func (Listed) reply()  {}
func (Failed) reply()  {}

func isTrue(ok bool) error {
	if !ok {
		return errors.New("must be true")
	}
	return nil
}

func isFalse(ok bool) error {
	if ok {
		return errors.New("must be false")
	}
	return nil
}

// Decode decodes data, one JSON document from outside the process, as a T, one
// of the package's wire types, and checks its rules: an unknown or missing
// member, a value of another JSON kind and a rule that a field breaks are an
// [*InvalidError], or several joined. The error names the field and the rule,
// never the value.
func Decode[T any](data []byte) (T, error) {
	var value T
	if err := json.Unmarshal(data, &value, wireOptions); err != nil {
		return value, wire.Refusal(err)
	}
	if err := check(value); err != nil {
		return value, err
	}
	return value, nil
}

// Encode returns the JSON of value, one of the package's wire types, after
// checking it as [Decode] would.
func Encode[T any](value T) ([]byte, error) {
	if err := check(value); err != nil {
		return nil, err
	}
	data, err := json.Marshal(value, wireOptions)
	if err != nil {
		return nil, wire.Refusal(err)
	}
	return data, nil
}

// check runs the generated rule check of a wire type. A value that is absent (a
// nil interface, or a nil pointer) is refused as required.
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
		return fmt.Errorf("claudeproto: %T is not a type of the wire", value)
	}
	return checked.validate()
}
