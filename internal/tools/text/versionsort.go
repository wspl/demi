package text

import (
	"regexp"
	"strings"
)

// compareVersions orders two strings as GNU sort -V does, following the
// rules in the coreutils manual (Version sort ordering): "", "." and ".."
// come first, then names starting with "." before others; a trailing file
// suffix is compared only when the rest is equal; runs of digits compare
// by value, and other characters compare with letters first, "~" before
// everything, even the end.
func compareVersions(a, b string) int {
	if a == b {
		return 0
	}
	for _, special := range []string{"", ".", ".."} {
		switch {
		case a == special:
			return -1
		case b == special:
			return 1
		}
	}
	hiddenA := strings.HasPrefix(a, ".")
	hiddenB := strings.HasPrefix(b, ".")
	if hiddenA != hiddenB {
		if hiddenA {
			return -1
		}
		return 1
	}
	if hiddenA {
		a = a[1:]
		b = b[1:]
	}
	baseA := a[:len(a)-fileSuffixLength(a)]
	baseB := b[:len(b)-fileSuffixLength(b)]
	if diff := compareVersionParts(baseA, baseB); diff != 0 {
		return diff
	}
	return compareVersionParts(a, b)
}

// fileSuffix matches a trailing file suffix such as ".tar.gz".
var fileSuffix = regexp.MustCompile(`(?:\.[A-Za-z~][A-Za-z0-9~]*)*$`)

// fileSuffixLength returns the length of the file suffix of s.
func fileSuffixLength(s string) int {
	match := fileSuffix.FindStringIndex(s)
	return len(s) - match[0]
}

// versionOrder is the weight of a non-digit character in a version string.
func versionOrder(s string, index int) int {
	if index >= len(s) {
		return 0
	}
	c := s[index]
	switch {
	case isDigit(c):
		return 0
	case isAlpha(c):
		return int(c)
	case c == '~':
		return -1
	}
	return int(c) + 256
}

// compareVersionParts compares alternating runs of non-digits and digits.
func compareVersionParts(a, b string) int {
	i, j := 0, 0
	for i < len(a) || j < len(b) {
		for (i < len(a) && !isDigit(a[i])) || (j < len(b) && !isDigit(b[j])) {
			orderA := versionOrder(a, i)
			orderB := versionOrder(b, j)
			if orderA != orderB {
				return compareInts(orderA, orderB)
			}
			i++
			j++
		}
		for i < len(a) && a[i] == '0' {
			i++
		}
		for j < len(b) && b[j] == '0' {
			j++
		}
		firstDiff := 0
		for i < len(a) && isDigit(a[i]) && j < len(b) && isDigit(b[j]) {
			if firstDiff == 0 {
				firstDiff = compareInts(int(a[i]), int(b[j]))
			}
			i++
			j++
		}
		if i < len(a) && isDigit(a[i]) {
			return 1
		}
		if j < len(b) && isDigit(b[j]) {
			return -1
		}
		if firstDiff != 0 {
			return firstDiff
		}
	}
	return 0
}
