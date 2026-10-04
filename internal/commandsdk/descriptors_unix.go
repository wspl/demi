//go:build darwin || linux

package commandsdk

import (
	"errors"
	"syscall"
)

// Exhausted reports a wrapped process or system open-file exhaustion.
func Exhausted(err error) bool {
	return errors.Is(err, syscall.EMFILE) || errors.Is(err, syscall.ENFILE)
}

// Exhaustion returns the platform's open-file exhaustion error.
func Exhaustion() error {
	return syscall.EMFILE
}
