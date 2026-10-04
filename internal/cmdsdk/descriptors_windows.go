package cmdsdk

import (
	"errors"

	"golang.org/x/sys/windows"
)

// Exhausted reports a wrapped open-file exhaustion.
func Exhausted(err error) bool {
	return errors.Is(err, windows.ERROR_TOO_MANY_OPEN_FILES)
}

// Exhaustion returns the platform's open-file exhaustion error.
func Exhaustion() error {
	return windows.ERROR_TOO_MANY_OPEN_FILES
}
