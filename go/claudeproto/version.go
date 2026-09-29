package claudeproto

import (
	"strconv"
	"strings"
)

// A Version is a SemVer 2.0 version without build metadata, such as 2.1.3 or
// 2.1.3-beta.1, as the Rust semver crate parses one. A version names its
// installation directory, so build metadata is refused.
type Version struct {
	Major, Minor, Patch uint64
	// Pre are the dot-separated identifiers of the pre-release part.
	Pre []string
}

// ParseVersion returns the version that s spells, or false when s is not one: it
// has three numbers without leading zeros that fit 64 bits, then optionally a
// hyphen and dot-separated pre-release identifiers of ASCII letters, digits and
// hyphens, none empty and no number with a leading zero.
func ParseVersion(s string) (Version, bool) {
	core, pre, hasPre := strings.Cut(s, "-")
	numbers := strings.Split(core, ".")
	if len(numbers) != 3 {
		return Version{}, false
	}
	var version Version
	for i, target := range []*uint64{&version.Major, &version.Minor, &version.Patch} {
		number, ok := parseNumber(numbers[i])
		if !ok {
			return Version{}, false
		}
		*target = number
	}
	if hasPre {
		version.Pre = strings.Split(pre, ".")
		for _, identifier := range version.Pre {
			if !validIdentifier(identifier) {
				return Version{}, false
			}
		}
	}
	return version, true
}

// IsVersion reports whether s is a version [ParseVersion] accepts.
func IsVersion(s string) bool {
	_, ok := ParseVersion(s)
	return ok
}

// parseNumber reads digits without a leading zero into 64 bits.
func parseNumber(s string) (uint64, bool) {
	if s == "" || len(s) > 1 && s[0] == '0' || strings.Trim(s, "0123456789") != "" {
		return 0, false
	}
	number, err := strconv.ParseUint(s, 10, 64)
	return number, err == nil
}

// validIdentifier reports whether s is a pre-release identifier.
func validIdentifier(s string) bool {
	if s == "" {
		return false
	}
	for _, c := range s {
		if c != '-' && (c < '0' || c > '9') && (c < 'A' || c > 'Z') && (c < 'a' || c > 'z') {
			return false
		}
	}
	// A number has no leading zero.
	return !numeric(s) || len(s) == 1 || s[0] != '0'
}

func numeric(identifier string) bool {
	return strings.Trim(identifier, "0123456789") == ""
}

// Compare returns -1, 0 or 1 as v has lower, equal or higher precedence than
// other (SemVer 2.0 § 11): the numbers first, then a version without a
// pre-release part above one with, then the pre-release identifiers one by one,
// numbers below words.
func (v Version) Compare(other Version) int {
	for _, pair := range [][2]uint64{{v.Major, other.Major}, {v.Minor, other.Minor}, {v.Patch, other.Patch}} {
		if pair[0] != pair[1] {
			if pair[0] < pair[1] {
				return -1
			}
			return 1
		}
	}
	switch {
	case len(v.Pre) == 0 && len(other.Pre) == 0:
		return 0
	case len(v.Pre) == 0:
		return 1
	case len(other.Pre) == 0:
		return -1
	}
	for i := range min(len(v.Pre), len(other.Pre)) {
		if c := compareIdentifiers(v.Pre[i], other.Pre[i]); c != 0 {
			return c
		}
	}
	return compareInts(len(v.Pre), len(other.Pre))
}

func compareIdentifiers(a, b string) int {
	switch {
	case numeric(a) && numeric(b):
		// Numbers compare by value; without leading zeros that is length, then
		// digits, so no number is too large.
		if c := compareInts(len(a), len(b)); c != 0 {
			return c
		}
		return strings.Compare(a, b)
	case numeric(a):
		return -1
	case numeric(b):
		return 1
	}
	return strings.Compare(a, b)
}

func compareInts(a, b int) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}
