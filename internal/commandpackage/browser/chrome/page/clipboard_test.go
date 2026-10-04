package page

import (
	"errors"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

// Clipboard diagnostics report the byte offset of invalid UTF-8 in text and HTML input.
// In-memory validation only; budget under one second.
func TestClipboardUTF8Diagnostic(t *testing.T) {
	for _, tc := range []struct {
		name  string
		mime  browserproto.ClipboardMIME
		input string
		want  string
	}{
		{
			"invalid_text",
			"text/plain",
			"é\xe1\x80!",
			"clipboard text is not UTF-8: invalid utf-8 sequence of 2 bytes from index 2",
		},
		{
			"incomplete_html",
			"text/html",
			"<p>é\xf1\x80",
			"clipboard text is not UTF-8: incomplete utf-8 byte sequence from index 5",
		},
		{"valid_text", "text/plain", "é😀�", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := validateClipboard(tc.mime, []byte(tc.input))
			if tc.want == "" {
				if err != nil {
					t.Fatal(err)
				}
				return
			}
			var failure *cdp.BrowserError
			if !errors.As(err, &failure) || failure.Kind != cdp.KindConfiguration {
				t.Fatalf("error = %v, want configuration error", err)
			}
			if failure.Message != tc.want {
				t.Fatalf("message = %q, want %q", failure.Message, tc.want)
			}
		})
	}
}
