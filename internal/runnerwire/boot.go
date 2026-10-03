package runnerwire

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf16"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/contract"
)

// A backend's URL (`runner.md` § Connection and identity): `http`, `https`,
// `ws` or `wss`, naming a host, without credentials or a fragment.
// +demi:codec
type BackendURL struct{ value string }

// ParseBackendURL checks and normalizes an installation's address.
func ParseBackendURL(value string) (BackendURL, error) {
	parsed, err := url.Parse(value)
	if err != nil {
		return BackendURL{}, fmt.Errorf("backend URL: %w", err)
	}
	if err := checkBackendURL(parsed); err != nil {
		return BackendURL{}, err
	}
	return BackendURL{value: parsed.Href(false)}, nil
}

// checkBackendURL checks the installation identity after WHATWG parsing.
func checkBackendURL(parsed *url.Url) error {
	scheme := parsed.Scheme()
	if (scheme != "http" && scheme != "https" && scheme != "ws" && scheme != "wss") || parsed.Hostname() == "" ||
		parsed.Username() != "" ||
		parsed.Password() != "" ||
		strings.Contains(parsed.Href(false), "#") {
		return fmt.Errorf("invalid backend URL")
	}
	return nil
}

// URL returns an independent parsed URL in its normal form.
func (u BackendURL) URL() (*url.Url, error) {
	parsed, err := url.Parse(u.value)
	if err != nil {
		return nil, fmt.Errorf("backend URL: %w", err)
	}
	if err := checkBackendURL(parsed); err != nil {
		return nil, err
	}
	return parsed, nil
}

// String returns the URL in its normal form, which names the installation.
func (u BackendURL) String() string { return u.value }

// A device's credential: 1 to 4096 characters, none of them whitespace. It
// never appears in debugging output.
// +demi:root
// +demi:id
// +demi:check validateDeviceToken
type DeviceToken string

// Expose returns the credential itself, for the header or file that carries it.
func (t DeviceToken) Expose() string { return string(t) }

// Format keeps credentials out of formatted diagnostics, including %+v and %#v.
func (DeviceToken) Format(state fmt.State, _ rune) {
	// fmt.State writes into fmt's own buffer and cannot fail.
	_, _ = state.Write([]byte("DeviceToken(..)"))
}

// validateDeviceToken preserves the credential's UTF-16 length bound.
func validateDeviceToken(token DeviceToken) error {
	size := 0
	for _, r := range string(token) {
		if unicode.IsSpace(r) {
			return fmt.Errorf("invalid device token")
		}
		size += utf16.RuneLen(r)
	}
	if size == 0 || size > 4096 {
		return fmt.Errorf("invalid device token")
	}
	return nil
}

// ManagedBoot holds the installation URL and credential read from a boot file.
// The URL is normalized at the generated raw record's conversion boundary.
// +demi:codec
type ManagedBoot struct {
	BackendURL  BackendURL
	DeviceToken DeviceToken
}

// +demi:root
type rawManagedBoot struct {
	BackendURL  string      `json:"backendUrl"`
	DeviceToken DeviceToken `json:"deviceToken"`
}

// DecodeManagedBoot decodes a boot file's JSON; its values are checked as they are read.
func DecodeManagedBoot(data []byte) (ManagedBoot, error) {
	raw, err := decodeRawManagedBoot(data)
	if err != nil {
		return ManagedBoot{}, fmt.Errorf("invalid managed boot file: %w", err)
	}
	backend, err := ParseBackendURL(raw.BackendURL)
	if err != nil {
		return ManagedBoot{}, fmt.Errorf("invalid managed boot file: %w", err)
	}
	return ManagedBoot{BackendURL: backend, DeviceToken: raw.DeviceToken}, nil
}

// MarshalJSON writes the boot record through its generated contract encoder.
func (b ManagedBoot) MarshalJSON() ([]byte, error) {
	if _, err := ParseBackendURL(b.BackendURL.value); err != nil {
		return nil, err
	}
	return contract.EncodeJSON(rawManagedBoot{BackendURL: b.BackendURL.value, DeviceToken: b.DeviceToken})
}

// UnmarshalJSON uses the same generated boundary as DecodeManagedBoot.
func (b *ManagedBoot) UnmarshalJSON(data []byte) error {
	value, err := DecodeManagedBoot(data)
	if err != nil {
		return err
	}
	*b = value
	return nil
}

// +demi:root
type rawBackendURL string

// MarshalJSON writes the normalized address as Rust's URL string.
func (u BackendURL) MarshalJSON() ([]byte, error) {
	if _, err := ParseBackendURL(u.value); err != nil {
		return nil, err
	}
	return contract.EncodeJSON(rawBackendURL(u.value))
}

// UnmarshalJSON parses the URL after its generated string decoder.
func (u *BackendURL) UnmarshalJSON(data []byte) error {
	raw, err := decodeRawBackendURL(data)
	if err != nil {
		return err
	}
	value, err := ParseBackendURL(string(raw))
	if err != nil {
		return err
	}
	*u = value
	return nil
}
