package cdp

import (
	"crypto/rand"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
)

// Fresh creates an unpredictable browser handle with the supplied prefix.
func Fresh(prefix string) (string, error) {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err != nil {
		return "", &BrowserError{Kind: KindIO, Cause: err}
	}
	return browserproto.Handle(prefix, bytes), nil
}
