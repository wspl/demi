package artifacts

import (
	"os"
	"path/filepath"
	"strings"
)

// Parent returns the path without its last component, like Rust's Path::parent.
// It preserves interior separators and .., so a symlink before .. is resolved
// by the filesystem. Trailing separators and non-leading dots are ignored.
// A relative single component has the empty parent and ok=true; roots, volume
// prefixes and the empty path have no parent (ok=false). Windows uses native
// prefixes and separators; verbatim paths retain dot components.
// When creating a relative file's empty parent with os.MkdirAll, use ".".
// Use this to find a file's parent before creating it; do not subsequently use
// filepath.Join, which would clean .. and could select a different directory.
func Parent(path string) (parent string, ok bool) {
	volume := filepath.VolumeName(path)
	start := len(volume)
	verbatim := os.PathSeparator == '\\' && strings.HasPrefix(path, `\\?\`)
	separator := func(c byte) bool {
		return c == byte(os.PathSeparator) || (!verbatim && os.PathSeparator == '\\' && c == '/')
	}
	root := start
	if root < len(path) && separator(path[root]) {
		root++
	}
	trim := func(end int) int {
		for end > root {
			if separator(path[end-1]) {
				end--
				continue
			}
			if !verbatim && path[end-1] == '.' && end-1 > start && separator(path[end-2]) {
				end--
				continue
			}
			break
		}
		return end
	}
	end := trim(len(path))
	if end <= root {
		return "", false
	}
	for end > root && !separator(path[end-1]) {
		end--
	}
	return path[:trim(end)], true
}

// artifactPath appends a relative artifact name without changing its parent's
// filesystem meaning. Artifact callers supply validated relative names.
func artifactPath(directory, name string) string {
	if directory == "" || strings.HasSuffix(directory, string(os.PathSeparator)) ||
		(os.PathSeparator == '\\' && strings.HasSuffix(directory, "/")) ||
		(len(directory) == 2 && directory[1] == ':' && os.PathSeparator == '\\') {
		return directory + name
	}
	return directory + string(os.PathSeparator) + name
}
