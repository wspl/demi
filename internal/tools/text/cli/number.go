package cli

import (
	"errors"
	"math"
	"math/bits"
	"strings"
	"syscall"
)

// ErrInvalidNumber is a number argument that is not a number.
var ErrInvalidNumber = errors.New("invalid number")

// ErrOverflow is a number argument too large for 64 bits; the parsed value
// is then the largest value. Its text is the C library's.
var ErrOverflow error = syscall.EOVERFLOW

// SizeSuffixes are the multiplier letters of the coreutils size options,
// with the SI forms (KB = 1000, KiB = 1024).
const SizeSuffixes = "bkKmMGTPEZYRQ0"

// ParseUint parses a decimal count with an optional multiplier suffix as
// coreutils' xstrtoumax does. suffixes lists the accepted letters; "0" among
// them also accepts "B" and "D" (powers of 1000) and "iB" (powers of 1024)
// after a power letter. A suffix without digits counts one.
func ParseUint(value, suffixes string) (uint64, error) {
	rest := strings.TrimLeft(value, " \t\n\v\f\r")
	if strings.HasPrefix(rest, "-") {
		return 0, ErrInvalidNumber
	}
	rest = strings.TrimPrefix(rest, "+")
	digits := 0
	for digits < len(rest) && rest[digits] >= '0' && rest[digits] <= '9' {
		digits++
	}
	var number uint64 = 1
	overflow := false
	if digits > 0 {
		number = 0
		for _, digit := range rest[:digits] {
			hi, lo := bits.Mul64(number, 10)
			sum, carry := bits.Add64(lo, uint64(digit-'0'), 0)
			if hi != 0 || carry != 0 {
				overflow = true
			}
			number = sum
		}
	} else if rest == "" || !strings.Contains(suffixes, rest[:1]) {
		return 0, ErrInvalidNumber
	}
	rest = rest[digits:]
	if rest != "" {
		letter := rest[0]
		if !strings.Contains(suffixes, string(letter)) || letter == '0' {
			return 0, ErrInvalidNumber
		}
		base := uint64(1024)
		rest = rest[1:]
		if strings.Contains(suffixes, "0") && strings.IndexByte("EGkKMmPQRTYZ", letter) >= 0 {
			switch {
			case strings.HasPrefix(rest, "iB"):
				rest = rest[2:]
			case strings.HasPrefix(rest, "B"), strings.HasPrefix(rest, "D"):
				base = 1000
				rest = rest[1:]
			}
		}
		if rest != "" {
			return 0, ErrInvalidNumber
		}
		var multiplied bool
		number, multiplied = multiply(number, letter, base)
		overflow = overflow || !multiplied
	}
	if overflow {
		return math.MaxUint64, ErrOverflow
	}
	return number, nil
}

// multiply applies the multiplier of a suffix letter; it reports false on
// overflow.
func multiply(number uint64, letter byte, base uint64) (uint64, bool) {
	var factor uint64
	power := 0
	switch letter {
	case 'b':
		factor = 512
	case 'c':
		factor = 1
	case 'w':
		factor = 2
	default:
		power = strings.IndexByte("kMGTPEZYRQ", letter) + 1
		if letter == 'K' {
			power = 1
		}
		if letter == 'm' {
			power = 2
		}
		factor = 1
	}
	for ; power > 0; power-- {
		hi, lo := bits.Mul64(factor, base)
		if hi != 0 {
			return math.MaxUint64, false
		}
		factor = lo
	}
	hi, lo := bits.Mul64(number, factor)
	if hi != 0 {
		return math.MaxUint64, false
	}
	return lo, true
}
