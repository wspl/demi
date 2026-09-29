// Package runnerproto is the contract between the runner and what starts it.
// It holds only the managed boot record so far (docs/cloud/managed-hosts.md
// § Managed boot credential): the machine manager writes it into a sandbox and
// the runner reads it before it connects. The rest of the runner protocol joins
// this package with the runner's port.
package runnerproto

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"errors"
	"net"
	"net/url"
	"strings"
	"unicode"
	"unicode/utf16"

	"github.com/wspl/demi/go/internal/wire"
)

// An InvalidError means a record breaks the rules of its wire type. It names the
// field and the rule, never the value: the device token is a secret.
type InvalidError = wire.InvalidError

// A BackendURL is a backend's URL as the wire carries it: http, https, ws or
// wss, naming a host, without credentials or a fragment.
type BackendURL string

// A DeviceToken is a device's credential: 1 to 4096 UTF-16 code units, none of
// them whitespace. It never appears in formatted output.
type DeviceToken string

// String hides the credential from formatted output; use [DeviceToken.Expose]
// for the header or file that carries it.
func (DeviceToken) String() string { return "DeviceToken(..)" }

// GoString hides the credential from %#v as well.
func (DeviceToken) GoString() string { return "DeviceToken(..)" }

// Expose returns the credential itself.
func (t DeviceToken) Expose() string { return string(t) }

// A ManagedBoot is the credential file a managed runner reads: where its backend
// is and the token of this boot. It refuses unknown fields.
//
//demi:wire
type ManagedBoot struct {
	BackendURL  BackendURL  `json:"backendUrl" check:"func=backendURL"`
	DeviceToken DeviceToken `json:"deviceToken" check:"func=deviceToken"`
}

// backendURL is the rule of a backend's URL.
func backendURL(value BackendURL) error {
	address, err := NormalURL(string(value))
	if err != nil {
		return errors.New("is not a URL")
	}
	switch address.Scheme {
	case "http", "https", "ws", "wss":
	default:
		return errors.New("is not a backend URL")
	}
	if address.Hostname() == "" || address.User != nil || address.Fragment != "" {
		return errors.New("is not a backend URL")
	}
	return nil
}

// deviceToken is the rule of a device token.
func deviceToken(value DeviceToken) error {
	length := len(utf16.Encode([]rune(string(value))))
	if length == 0 || length > 4096 || strings.IndexFunc(string(value), unicode.IsSpace) >= 0 {
		return errors.New("is not a device token")
	}
	return nil
}

// NormalURL parses raw as an absolute URL and returns it in its normal form,
// which names an installation: the scheme and the host in lower case, no port
// that is the scheme's default, and "/" for an empty path, as the WHATWG URL
// standard writes them, so two spellings of one endpoint compare equal.
func NormalURL(raw string) (*url.URL, error) {
	address, err := url.Parse(raw)
	if err != nil {
		return nil, err
	}
	if address.Scheme == "" || address.Opaque != "" {
		return nil, errors.New("is not an absolute URL")
	}
	address.Scheme = strings.ToLower(address.Scheme)
	host := strings.ToLower(address.Hostname())
	port := address.Port()
	if port == defaultPorts[address.Scheme] {
		port = ""
	}
	switch {
	case port != "":
		address.Host = net.JoinHostPort(host, port)
	case strings.Contains(host, ":"):
		address.Host = "[" + host + "]"
	default:
		address.Host = host
	}
	if address.Host != "" && address.Path == "" {
		address.Path = "/"
	}
	return address, nil
}

// defaultPorts are the ports the special schemes leave out.
var defaultPorts = map[string]string{"http": "80", "https": "443", "ws": "80", "wss": "443"}

// Normal returns the URL in its normal form, or an error when it is not a
// backend URL.
func (u BackendURL) Normal() (string, error) {
	if err := backendURL(u); err != nil {
		return "", err
	}
	address, err := NormalURL(string(u))
	if err != nil {
		return "", err
	}
	return address.String(), nil
}

// DecodeManagedBoot decodes a boot file's JSON and checks its values.
func DecodeManagedBoot(data []byte) (ManagedBoot, error) {
	return decode[ManagedBoot](data)
}

// Encode returns the boot file's JSON, with the URL in its normal form.
func (b ManagedBoot) Encode() ([]byte, error) {
	normal, err := b.BackendURL.Normal()
	if err != nil {
		return nil, &InvalidError{Path: "backendUrl", Rule: "is not a backend URL"}
	}
	b.BackendURL = BackendURL(normal)
	return encode(b)
}

// Validate checks value, one of the package's wire types, against the rules of
// its fields, as decoding does. A wire type of another package holds one of
// these values and names this function in a func rule to run its rules.
func Validate[T any](value T) error {
	return check(value)
}
