package plugin

import (
	"crypto/sha256"
	"fmt"
	"slices"
	"strings"
)

// Digest hashes the sorted mode, blob hash and path listing.
func (d HostDirectory) Digest() string {
	files := slices.Clone(d.Files)
	slices.SortStableFunc(files, func(a, b DirectoryFile) int {
		return strings.Compare(a.Path, b.Path)
	})
	var listing strings.Builder
	for _, f := range files {
		mode := "644"
		if f.Executable {
			mode = "755"
		}
		fmt.Fprintf(&listing, "%s %s %s\n", mode, f.Blob, f.Path)
	}
	return fmt.Sprintf("%x", sha256.Sum256([]byte(listing.String())))
}

// HostName names the directory's contents on a Host.
func (d HostDirectory) HostName() string {
	return d.Name + "-" + d.Digest()[:12]
}

// Path locates the directory below the Host's home.
func (d HostDirectory) Path(plugin ID) string {
	return "~/.demi/plugins/" + string(plugin) + "/" + d.HostName()
}

// CheckDirectories refuses duplicate names and invalid or duplicate paths.
func CheckDirectories(directories []HostDirectory) error {
	names := make(map[string]bool)
	for _, d := range directories {
		if !validName(d.Name, 64) {
			return fmt.Errorf("\"%s\" is not a directory name: 1 to 64 lowercase letters, digits and hyphens", d.Name)
		}
		if names[d.Name] {
			return fmt.Errorf("the directory \"%s\" is named twice", d.Name)
		}
		names[d.Name] = true
		paths := make(map[string]bool)
		for _, f := range d.Files {
			for _, part := range strings.Split(f.Path, "/") {
				if part == "" || part == "." || part == ".." {
					return fmt.Errorf("\"%s\" in \"%s\" is not a relative path", f.Path, d.Name)
				}
			}
			if paths[f.Path] {
				return fmt.Errorf("\"%s\" is in \"%s\" twice", f.Path, d.Name)
			}
			paths[f.Path] = true
		}
	}
	return nil
}

// validName checks the plugin contract's lowercase directory and identifier alphabet.
func validName(name string, limit int) bool {
	if len(name) < 1 || len(name) > limit {
		return false
	}
	for _, c := range []byte(name) {
		if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '-' {
			return false
		}
	}
	return true
}
