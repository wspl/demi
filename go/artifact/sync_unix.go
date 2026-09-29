//go:build unix

package artifact

import "os"

// syncDirectory puts a directory's entries on disk.
func syncDirectory(path string) error {
	directory, err := os.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
