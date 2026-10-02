package contract

import (
	"errors"
	"fmt"
	"regexp"
	"strings"
	"unicode"

	"github.com/nlnwa/whatwg-url/url"
)

// Trim removes the whitespace JavaScript and the contract's text types trim.
// U+0085 is Unicode whitespace but is deliberately not JavaScript whitespace.
func Trim(value string) string {
	return strings.TrimFunc(value, func(r rune) bool {
		return r == '\ufeff' || unicode.IsSpace(r) && r != '\u0085'
	})
}

var emailPattern = regexp.MustCompile(`^[A-Za-z0-9_'+\-.]*[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$`)

// Email canonicalizes and checks the address spelling used for storage lookups.
func Email(value string) (string, error) {
	// Rust uses full Unicode lowercase. U+0130 expands to i plus a combining
	// dot, while Go's simple lowercase would turn it into an accepted ASCII i.
	value = strings.ReplaceAll(value, "\u0130", "i\u0307")
	value = strings.ToLower(Trim(value))
	if err := Text(value, 0, 254, ""); err != nil {
		return "", err
	}
	if strings.HasPrefix(value, ".") || strings.Contains(value, "..") || !emailPattern.MatchString(value) {
		return "", errors.New("must be an email address")
	}
	return value, nil
}

// HTTPURL parses an HTTP(S) endpoint and returns its WHATWG serialization.
func HTTPURL(value string) (string, error) {
	if err := Text(value, 0, -1, ""); err != nil {
		return "", err
	}
	parsed, err := url.Parse(value)
	if err != nil {
		return "", fmt.Errorf("must be an http or https URL: %w", err)
	}
	if (parsed.Scheme() != "http" && parsed.Scheme() != "https") || parsed.Hostname() == "" {
		return "", errors.New("must be an http or https URL")
	}
	return parsed.Href(false), nil
}
