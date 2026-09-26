package zodrt

import (
	"math"
	"strings"
	"time"
	"unicode/utf8"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// maxDateMillis is the largest distance from the epoch a JavaScript Date
// holds, in milliseconds.
const maxDateMillis = 8_640_000_000_000_000

// CheckFinite rejects NaN and the infinities, which `z.number()` rejects.
func CheckFinite(number float64) error {
	if math.IsNaN(number) || math.IsInf(number, 0) {
		return Invalid("number must be finite")
	}
	return nil
}

// CheckSafeInt rejects an integer outside JavaScript's safe range.
func CheckSafeInt(number int64) error {
	if number > maxSafeInteger || number < -maxSafeInteger {
		return Invalid("integer out of range")
	}
	return nil
}

// Length is a string's length as Zod measures it: in Unicode code points.
func Length(text string) int {
	return utf8.RuneCountInString(text)
}

// jsWhitespace is what JavaScript's String.prototype.trim removes.
const jsWhitespace = "\t\n\v\f\r \u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"

// CheckURL accepts what `z.url()` accepts: a string the WHATWG URL parser
// (JavaScript's `new URL`) parses once surrounding whitespace is trimmed.
func CheckURL(text string) error {
	if _, err := whatwg.Parse(strings.Trim(text, jsWhitespace)); err != nil {
		return Invalid("invalid URL")
	}
	return nil
}

// CheckDate rejects a time a JavaScript Date cannot hold.
func CheckDate(instant time.Time) error {
	if instant.Before(time.UnixMilli(-maxDateMillis)) || instant.After(time.UnixMilli(maxDateMillis)) {
		return Invalid("date out of range")
	}
	return nil
}

// Unique rejects an array with repeated items.
func Unique[T comparable](items []T) error {
	seen := make(map[T]struct{}, len(items))
	for _, item := range items {
		if _, ok := seen[item]; ok {
			return Invalid("array items must be unique")
		}
		seen[item] = struct{}{}
	}
	return nil
}
