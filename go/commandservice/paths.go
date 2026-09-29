package commandservice

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"
)

// ErrInvalidPath is the error of a path that names nothing: it is empty or holds
// a NUL byte.
var ErrInvalidPath = errors.New("path must be nonempty and contain no NUL byte")

// ResolvePath returns path joined to cwd, the working directory an invocation
// names, never the process's own. An absolute path stays as it is. The result is
// not cleaned: a ".." in it is left for the system to follow, since removing it
// changes what a path through a symbolic link names. A path that is empty or
// holds a NUL byte names nothing and is refused with [ErrInvalidPath].
func ResolvePath(cwd, path string) (string, error) {
	if path == "" || strings.IndexByte(path, 0) >= 0 {
		return "", ErrInvalidPath
	}
	switch {
	case filepath.IsAbs(path), filepath.VolumeName(path) != "":
		return path, nil
	case os.IsPathSeparator(path[0]):
		// A path with a root and no volume (Windows) keeps the directory's
		// volume.
		return filepath.VolumeName(cwd) + path, nil
	case cwd == "":
		return path, nil
	case os.IsPathSeparator(cwd[len(cwd)-1]):
		return cwd + path, nil
	}
	return cwd + string(filepath.Separator) + path, nil
}

// PathKey returns what a comparison of paths looks at, as Rust's path equality
// does: the path without its repeated separators and its "." components, except
// a "." that starts a relative path. It does not follow ".." (which may name
// another place through a link), so two paths that name one file may differ, and
// paths with one key are equal. Use it to ask whether two paths are the same, or
// as a map key.
func PathKey(path string) string {
	volume := filepath.VolumeName(path)
	rest := path[len(volume):]
	rooted := rest != "" && os.IsPathSeparator(rest[0])
	var parts []string
	for i, part := range strings.FieldsFunc(rest, func(r rune) bool { return r < utf8.RuneSelf && os.IsPathSeparator(byte(r)) }) {
		if part != "." || i == 0 && !rooted {
			parts = append(parts, part)
		}
	}
	key := volume
	if rooted {
		key += string(filepath.Separator)
	}
	return key + strings.Join(parts, string(filepath.Separator))
}

// normalizedAbsolute makes path absolute, with the process's working directory
// when it is not, and gives it its [PathKey]; it follows neither links nor "..".
func normalizedAbsolute(path string) string {
	if !filepath.IsAbs(path) {
		if cwd, err := os.Getwd(); err == nil {
			path = cwd + string(filepath.Separator) + path
		}
	}
	return PathKey(path)
}

// ParentPath returns the path without its last component, as Rust's parent does
// for the path's components: "a/b" has the parent "a", "a" the empty path, and a
// root or the empty path has none. It does not follow "..", which is a component
// like another.
func ParentPath(path string) (string, bool) {
	key := PathKey(path)
	volume := filepath.VolumeName(key)
	rest := key[len(volume):]
	for end := len(rest) - 1; end >= 0; end-- {
		if !os.IsPathSeparator(rest[end]) {
			continue
		}
		if end == 0 {
			// The root holds what is left of a path with one component; a
			// root itself has no parent.
			if len(rest) == 1 {
				return "", false
			}
			return key[:len(volume)+1], true
		}
		return key[:len(volume)+end], true
	}
	if rest == "" {
		return "", false
	}
	return volume, true
}
