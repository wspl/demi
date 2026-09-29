// Package hostpath interprets a Host path without using the backend's OS.
package hostpath

import "strings"

// Absolute follows typed_path::Utf8TypedPath::derive(...).is_absolute().
// Windows requires a parsed prefix followed by a root separator; a bare UNC
// share is therefore not absolute, even though the Windows API may treat it so.
func Absolute(text string) bool {
	rest, prefix := windowsPrefix(text)
	if prefix {
		separators := `\/`
		if strings.HasPrefix(text, `\\?\`) {
			separators = `\`
		}
		return rest != "" && strings.ContainsRune(separators, rune(rest[0]))
	}
	return strings.HasPrefix(text, "/")
}

// windowsPrefix consumes a typed-path Windows prefix, leaving its root intact.
func windowsPrefix(text string) (string, bool) {
	if drivePrefix(text) {
		return text[2:], true
	}
	if len(text) < 2 || !strings.ContainsRune(`\/`, rune(text[0])) || !strings.ContainsRune(`\/`, rune(text[1])) {
		return "", false
	}
	if len(text) >= 4 && text[2] == '?' && strings.ContainsRune(`\/`, rune(text[3])) {
		rest := text[4:]
		separators := `\/`
		if strings.HasPrefix(text, `\\?\`) {
			separators = `\`
		}
		if len(rest) >= 4 && rest[:3] == "UNC" && strings.ContainsRune(separators, rune(rest[3])) {
			if tail, ok := uncPrefix(rest[4:], separators); ok {
				return tail, true
			}
		}
		if drivePrefix(rest) {
			return rest[2:], true
		}
		if rest == "" {
			return "", false
		}
		end := strings.IndexAny(rest, separators)
		if end < 0 {
			return "", true
		}
		return rest[end:], true
	}
	if len(text) >= 4 && text[2] == '.' && strings.ContainsRune(`\/`, rune(text[3])) {
		rest := text[4:]
		end := strings.IndexAny(rest, `\/`)
		if rest != "" && end != 0 {
			if end < 0 {
				return "", true
			}
			return rest[end:], true
		}
	}
	return uncPrefix(text[2:], `\/`)
}

// uncPrefix consumes the server and optional share of a Host's UNC prefix.
func uncPrefix(text, separators string) (string, bool) {
	if text == "" {
		return "", false
	}
	end := strings.IndexAny(text, separators)
	if end == 0 {
		return "", false
	}
	if end < 0 {
		return "", true
	}
	rest := text[end+1:]
	end = strings.IndexAny(rest, separators)
	if end < 0 {
		return "", true
	}
	return rest[end:], true
}

// drivePrefix recognizes the ASCII drive letters typed-path recognizes.
func drivePrefix(text string) bool {
	return len(text) >= 2 && text[1] == ':' && (text[0] >= 'a' && text[0] <= 'z' || text[0] >= 'A' && text[0] <= 'Z')
}
