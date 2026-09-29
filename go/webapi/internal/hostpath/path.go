// Package hostpath interprets a Host path without using the backend's OS.
package hostpath

import "strings"

// Absolute follows typed_path::Utf8TypedPath::derive(text).is_absolute(). A
// path is a Windows one when it starts with a backslash or a Windows prefix
// parses, and is then absolute when the prefix is followed by a root
// separator; so a bare UNC share is not absolute, even though the Windows API
// may treat it so. Any other path is a Unix one, absolute when it starts with
// "/".
func Absolute(text string) bool {
	rest, ok := prefix(text)
	if !ok {
		return !strings.HasPrefix(text, `\`) && strings.HasPrefix(text, "/")
	}
	// A path that starts with exactly \\?\ is verbatim: only "\" separates.
	normalize := !strings.HasPrefix(text, `\\?\`)
	return rest != "" && separator(rest[0], normalize)
}

// separator reports whether b separates components: "\", and "/" too when the
// path is normalized.
func separator(b byte, normalize bool) bool {
	return b == '\\' || normalize && b == '/'
}

// prefix parses a Windows prefix as typed_path does, trying its forms in its
// order, and returns what follows it.
func prefix(text string) (string, bool) {
	for _, form := range []func(string) (string, bool){verbatimUNC, verbatimDisk, verbatim, deviceNS, unc, disk} {
		if rest, ok := form(text); ok {
			return rest, true
		}
	}
	return "", false
}

// verbatimUNC is \\?\UNC\SERVER\SHARE, the share optional.
func verbatimUNC(text string) (string, bool) {
	normalize := !strings.HasPrefix(text, `\\?\`)
	rest, ok := verbatimStart(text)
	if !ok || !strings.HasPrefix(rest, "UNC") {
		return "", false
	}
	rest = rest[3:]
	if rest == "" || !separator(rest[0], normalize) {
		return "", false
	}
	rest, ok = component(rest[1:], normalize)
	if !ok {
		return "", false
	}
	return optionalShare(rest, normalize), true
}

// verbatimDisk is \\?\C:.
func verbatimDisk(text string) (string, bool) {
	rest, ok := verbatimStart(text)
	if !ok {
		return "", false
	}
	return disk(rest)
}

// verbatim is \\?\NAME, or \\?\ before a separator (a blank name). The two
// verbatim forms above are tried first, as typed_path excludes them here.
func verbatim(text string) (string, bool) {
	normalize := !strings.HasPrefix(text, `\\?\`)
	rest, ok := verbatimStart(text)
	if !ok {
		return "", false
	}
	if name, ok := component(rest, normalize); ok {
		return name, true
	}
	if rest != "" && separator(rest[0], normalize) {
		return rest, true
	}
	return "", false
}

// deviceNS is \\.\DEVICE.
func deviceNS(text string) (string, bool) {
	if len(text) < 4 || !separator(text[0], true) || !separator(text[1], true) || text[2] != '.' || !separator(text[3], true) {
		return "", false
	}
	return component(text[4:], true)
}

// unc is \\SERVER\SHARE, the share optional.
func unc(text string) (string, bool) {
	if len(text) < 2 || !separator(text[0], true) || !separator(text[1], true) {
		return "", false
	}
	rest, ok := component(text[2:], true)
	if !ok {
		return "", false
	}
	return optionalShare(rest, true), true
}

// disk is C:, the drive an ASCII letter: a string's other letters are longer
// than a byte, and typed_path takes the drive as one.
func disk(text string) (string, bool) {
	if len(text) < 2 || text[1] != ':' || !(text[0] >= 'a' && text[0] <= 'z' || text[0] >= 'A' && text[0] <= 'Z') {
		return "", false
	}
	return text[2:], true
}

// verbatimStart consumes \\?\, each separator either slash.
func verbatimStart(text string) (string, bool) {
	if len(text) < 4 || !separator(text[0], true) || !separator(text[1], true) || text[2] != '?' || !separator(text[3], true) {
		return "", false
	}
	return text[4:], true
}

// component consumes the bytes up to the next separator, at least one.
func component(text string, normalize bool) (string, bool) {
	end := 0
	for end < len(text) && !separator(text[end], normalize) {
		end++
	}
	return text[end:], end > 0
}

// optionalShare consumes the separator and the share that may follow a server.
func optionalShare(rest string, normalize bool) string {
	if rest != "" && separator(rest[0], normalize) {
		rest = rest[1:]
	}
	if tail, ok := component(rest, normalize); ok {
		return tail
	}
	return rest
}
