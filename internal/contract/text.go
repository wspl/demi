package contract

import (
	"errors"
	"regexp"
	"strings"
	"unicode"
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
