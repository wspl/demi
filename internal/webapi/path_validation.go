package webapi

import (
	"errors"
	"regexp"
	"strings"
)

func validateTreePath(path TreePath) error {
	if path == "" || strings.HasPrefix(string(path), "/") {
		return errors.New("path must be a relative path inside the working tree")
	}
	for segment := range strings.SplitSeq(string(path), "/") {
		if segment == ".." {
			return errors.New("path must be a relative path inside the working tree")
		}
	}
	return nil
}

// A Host path may start with a Windows prefix even on Unix, tried in this order:
// \\?\UNC\server\share, \\?\C:, \\?\name, \\.\device, \\server\share, C:.
// The standard filepath package only recognizes the build target's paths.
var hostPrefixes = []*regexp.Regexp{
	regexp.MustCompile(`^[\\/]{2}\?[\\/]UNC[\\/][^\\/]+(?:[\\/][^\\/]*)?`),
	regexp.MustCompile(`^[\\/]{2}\?[\\/][A-Za-z]:`),
	regexp.MustCompile(`^[\\/]{2}\?[\\/][^\\/]*`),
	regexp.MustCompile(`^[\\/]{2}\.[\\/][^\\/]+`),
	regexp.MustCompile(`^[\\/]{2}[^\\/]+(?:[\\/][^\\/]*)?`),
	regexp.MustCompile(`^[A-Za-z]:`),
}

var verbatimHostPrefixes = []*regexp.Regexp{
	regexp.MustCompile(`^\\\\\?\\UNC\\[^\\]+(?:\\[^\\]*)?`),
	regexp.MustCompile(`^\\\\\?\\[A-Za-z]:`),
	regexp.MustCompile(`^\\\\\?\\[^\\]*`),
}

func validateAbsolutePath(path AbsolutePath) error {
	s := string(path)
	prefixes := hostPrefixes
	verbatim := strings.HasPrefix(s, `\\?\`)
	if verbatim {
		prefixes = verbatimHostPrefixes
	}
	for _, pattern := range prefixes {
		prefix := pattern.FindString(s)
		if prefix == "" {
			continue
		}
		if len(prefix) < len(s) && (s[len(prefix)] == '\\' || !verbatim && s[len(prefix)] == '/') {
			return nil
		}
		return errors.New("path must be an absolute path on the Host")
	}
	if strings.HasPrefix(s, "/") {
		return nil
	}
	return errors.New("path must be an absolute path on the Host")
}
