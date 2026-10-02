package cdp

import (
	"crypto/rand"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Fresh creates an unpredictable browser handle with the supplied prefix.
func Fresh(prefix string) (string, error) {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err != nil {
		return "", &BrowserError{Kind: KindIO, Cause: err}
	}
	return browserop.Handle(prefix, bytes), nil
}
