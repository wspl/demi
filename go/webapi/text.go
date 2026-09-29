package webapi

import (
	"encoding/json/jsontext"
	"regexp"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/webapi/internal/endpoint"
)

const EmailMax = 254

// EmailAddress stores the spelling used for account lookup and uniqueness.
//
//demi:opaque string format=email
//wiregen:browser inline {"type":"string","maxLength":254}
type EmailAddress struct{ core.Identity[EmailKind] }
type EmailKind struct{}

var emailPattern = regexp.MustCompile(`^[A-Za-z0-9_'+\-.]*[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$`)

func (EmailKind) CheckIdentity(text string) error {
	if utf8.RuneCountInString(text) > EmailMax {
		return &InvalidError{Rule: "must be at most 254 characters"}
	}
	if strings.HasPrefix(text, ".") || strings.Contains(text, "..") || !emailPattern.MatchString(text) {
		return &InvalidError{Rule: "must be an email address"}
	}
	return nil
}
func ParseEmailAddress(text string) (EmailAddress, error) {
	id, err := core.ParseIdentity[EmailKind](strings.ToLower(core.Trim(text)))
	return EmailAddress{Identity: id}, err
}
func (v EmailAddress) validate() error { return core.Validate(v.Identity) }
func (v *EmailAddress) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	next, err := ParseEmailAddress(text)
	if err != nil {
		return err
	}
	*v = next
	return nil
}

// Trimmed removes JavaScript whitespace before a field checks its bounds.
//
//wiregen:browser inline {"type":"string","format":"trimmed"}
type Trimmed string

func NewTrimmed(text string) Trimmed { return Trimmed(core.Trim(text)) }
func (v Trimmed) String() string     { return string(v) }
func (v Trimmed) validate() error {
	if string(v) != core.Trim(string(v)) {
		return &InvalidError{Rule: "must be trimmed"}
	}
	return nil
}
func (v *Trimmed) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	*v = NewTrimmed(text)
	return nil
}

// Password redacts formatting; Expose is the explicit access to its contents.
//
//wiregen:browser inline {"type":"string"}
type Password string

func (Password) String() string   { return "Password(..)" }
func (Password) GoString() string { return "Password(..)" }
func (v Password) Expose() string { return string(v) }
func (Password) validate() error  { return nil }

// EndpointURL is an HTTP or HTTPS endpoint in its normal form.
//
//demi:opaque string format=http-url
//wiregen:browser inline {"type":"string"}
type EndpointURL struct{ core.Identity[EndpointKind] }
type EndpointKind struct{}

func (EndpointKind) CheckIdentity(text string) error {
	normalized, err := endpoint.Parse(text)
	if err != nil {
		return err
	}
	if normalized != text {
		return &InvalidError{Rule: "must be a normalized endpoint URL"}
	}
	return nil
}
func ParseEndpointURL(text string) (EndpointURL, error) {
	normalized, err := endpoint.Parse(text)
	if err != nil {
		return EndpointURL{}, err
	}
	id, err := core.ParseIdentity[EndpointKind](normalized)
	return EndpointURL{Identity: id}, err
}
func (v *EndpointURL) UnmarshalText(text []byte) error {
	next, err := ParseEndpointURL(string(text))
	if err != nil {
		return err
	}
	*v = next
	return nil
}
func (v EndpointURL) validate() error { return core.Validate(v.Identity) }
func (v *EndpointURL) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	next, err := ParseEndpointURL(text)
	if err != nil {
		return err
	}
	*v = next
	return nil
}
