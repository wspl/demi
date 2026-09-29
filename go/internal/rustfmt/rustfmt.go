// Package rustfmt formats and reads values as the Rust implementation does where a
// user or the model reads the result: a float in a JSON document, a float in text,
// and a string in quotes (Rust's {:?}); and the numbers a command's argument names.
// The formats are part of what a command prints and accepts, so they follow the
// Rust ones digit for digit, not Go's.
package rustfmt

import (
	"math"
	"regexp"
	"strconv"
	"strings"
	"unicode"
)

// JSONFloat returns f as serde_json prints it: the shortest digits that read
// back as f, in decimal notation with a ".0" for a whole number when the value
// lies from 1e-5 up to below 1e16, and otherwise in scientific notation such as
// "1e30" or "1.234e-7". f must be finite.
func JSONFloat(f float64) string {
	if f == 0 {
		if math.Signbit(f) {
			return "-0.0"
		}
		return "0.0"
	}
	sign := ""
	if f < 0 {
		sign = "-"
		f = -f
	}
	// digits and exponent of the shortest form d.ddd × 10^exponent.
	scientific := strconv.FormatFloat(f, 'e', -1, 64)
	mantissa, exponentText, _ := strings.Cut(scientific, "e")
	digits := strings.Replace(mantissa, ".", "", 1)
	exponent, _ := strconv.Atoi(exponentText)
	length := len(digits)
	// The value is digits × 10^k, and 10^(kk-1) <= value < 10^kk.
	k := exponent - (length - 1)
	kk := length + k
	switch {
	case 0 <= k && kk <= 16:
		return sign + digits + strings.Repeat("0", kk-length) + ".0"
	case 0 < kk && kk <= 16:
		return sign + digits[:kk] + "." + digits[kk:]
	case -5 < kk && kk <= 0:
		return sign + "0." + strings.Repeat("0", -kk) + digits
	case length == 1:
		return sign + digits + "e" + strconv.Itoa(kk-1)
	default:
		return sign + digits[:1] + "." + digits[1:] + "e" + strconv.Itoa(kk-1)
	}
}

// DisplayFloat returns f as Rust's Display prints it: the shortest digits that
// read back as f, never in scientific notation, and a whole number without a
// fractional part ("360", "0.5").
func DisplayFloat(f float64) string {
	return strconv.FormatFloat(f, 'f', -1, 64)
}

// EscapeDebug returns r as Rust's char::escape_debug writes it, the way a
// control character is made visible: "\n", "\r", "\t", "\\", "\"", "'" and
// "\0" by name, a character that is not printable or extends a grapheme as
// "\u{1b}", and anything else as it is.
func EscapeDebug(r rune) string {
	switch r {
	case 0:
		return `\0`
	case '\t':
		return `\t`
	case '\r':
		return `\r`
	case '\n':
		return `\n`
	case '\\':
		return `\\`
	case '"':
		return `\"`
	case '\'':
		return `\'`
	}
	if isGraphemeExtend(r) || !unicode.IsPrint(r) {
		return `\u{` + strconv.FormatInt(int64(r), 16) + `}`
	}
	return string(r)
}

// isGraphemeExtend reports whether r has the Unicode property Grapheme_Extend:
// the nonspacing and enclosing marks and a few more characters.
func isGraphemeExtend(r rune) bool {
	return unicode.In(r, unicode.Mn, unicode.Me, unicode.Other_Grapheme_Extend)
}

// Quote returns s in double quotes as Rust's {:?} writes a str: a double quote
// and a backslash are escaped and so are the characters [EscapeDebug] escapes,
// except the single quote, which stays.
func Quote(s string) string {
	var out strings.Builder
	out.WriteByte('"')
	for _, r := range s {
		if r == '\'' {
			out.WriteByte('\'')
			continue
		}
		out.WriteString(EscapeDebug(r))
	}
	out.WriteByte('"')
	return out.String()
}

// floatSyntax is what Rust's f64 FromStr accepts: an optional sign, then decimal
// digits with an optional point and exponent, or inf, infinity or nan in any case.
// Go's [strconv.ParseFloat] also takes hexadecimal floats and underscores.
var floatSyntax = regexp.MustCompile(`^(?i:[+-]?(?:(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:e[+-]?[0-9]+)?|inf|infinity|nan))$`)

// ParseFloat reads s as Rust's str::parse::<f64> does. A value out of range is
// the infinity of its sign, not an error.
func ParseFloat(s string) (float64, bool) {
	if !floatSyntax.MatchString(s) {
		return 0, false
	}
	f, err := strconv.ParseFloat(s, 64)
	if err != nil && !math.IsInf(f, 0) {
		return 0, false
	}
	return f, true
}
