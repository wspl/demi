// Package fsfail words the failure of a file operation for a message that
// names the path itself.
package fsfail

import (
	"errors"
	"io/fs"
)

// Cause is the cause of a failed file operation, without the path an
// *fs.PathError repeats, for a message that names the path itself.
func Cause(err error) error {
	var failed *fs.PathError
	if errors.As(err, &failed) {
		return failed.Err
	}
	return err
}
