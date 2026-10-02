package cli

import (
	"slices"
	"strings"
	"unicode/utf8"
)

// UnknownVariable returns the name of the first variable in environ whose
// name starts with prefix but is not among names, and whether one was found.
// If none was found, it returns "", false. Callers choose their own diagnostic.
// names contains the full environment variable names the program reads;
// environ contains NAME=value entries, as returned by os.Environ.
// Matching is case-sensitive. Values are not inspected, and names that are not
// valid UTF-8 are ignored, matching the shared CLI's Unicode setting names.
func UnknownVariable(prefix string, names, environ []string) (name string, found bool) {
	for _, entry := range environ {
		name, _, _ := strings.Cut(entry, "=")
		if utf8.ValidString(name) && strings.HasPrefix(name, prefix) && !slices.Contains(names, name) {
			return name, true
		}
	}
	return "", false
}
