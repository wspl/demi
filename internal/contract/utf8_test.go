package contract_test

import (
	"errors"
	"testing"

	"github.com/wspl/demi/internal/contract"
)

// Rust std::str::from_utf8 diagnostics pin malformed widths and byte offsets.
// In-memory table only; budget <1 second.
func TestCheckUTF8(t *testing.T) {
	cases := []struct {
		name, input, want string
		index, width      int
	}{
		{"empty", "", "", 0, 0},
		{"valid", "ASCII\x00é中😀�", "", 0, 0},
		{"truncated two", "\xc2", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"truncated three first", "\xe1", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"truncated three second", "\xe1\x80", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"truncated four first", "\xf1", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"truncated four second", "\xf1\x80", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"truncated four third", "\xf1\x80\x80", "incomplete utf-8 byte sequence from index 0", 0, 0},
		{"overlong two", "\xc0\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"overlong three", "\xe0\x9f\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"overlong four", "\xf0\x8f\x80\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"surrogate", "\xed\xa0\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"above maximum", "\xf4\x90\x80\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"invalid lead", "\xf5\x80\x80\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"continuation", "\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"invalid second", "\xc2!", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"invalid third", "\xe1\x80!", "invalid utf-8 sequence of 2 bytes from index 0", 0, 2},
		{"invalid fourth", "\xf1\x80\x80!", "invalid utf-8 sequence of 3 bytes from index 0", 0, 3},
		{"invalid before truncation", "\xe0\x80", "invalid utf-8 sequence of 1 bytes from index 0", 0, 1},
		{"valid prefix", "aé😀\xe1\x80!", "invalid utf-8 sequence of 2 bytes from index 7", 7, 2},
		{"incomplete after prefix", "aé😀\xf1\x80", "incomplete utf-8 byte sequence from index 7", 7, 0},
		{"first error only", "ok\xff\x80", "invalid utf-8 sequence of 1 bytes from index 2", 2, 1},
		{"valid boundaries", "\xc2\x80\xdf\xbf\xe0\xa0\x80\xed\x9f\xbf\xee\x80\x80\xef\xbf\xbf\xf0\x90\x80\x80\xf4\x8f\xbf\xbf", "", 0, 0},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := contract.CheckUTF8([]byte(tc.input))
			if tc.want == "" {
				if err != nil {
					t.Fatal(err)
				}
				return
			}
			if err == nil || err.Error() != tc.want {
				t.Fatalf("CheckUTF8 = %v; want %q", err, tc.want)
			}
			var invalid *contract.UTF8Error
			if !errors.As(err, &invalid) || invalid.ValidUpTo != tc.index || invalid.ErrorLen != tc.width {
				t.Fatalf("CheckUTF8 details = %#v; want index %d, width %d", invalid, tc.index, tc.width)
			}
		})
	}
}

// Expected strings are Rust String::from_utf8_lossy results, including one
// replacement for a truncated valid prefix and separate replacements for
// adjacent invalid leads. In-memory table; budget <1 second.
func TestLossyUTF8(t *testing.T) {
	for _, tc := range []struct{ name, input, want string }{
		{"empty", "", ""},
		{"valid", "ASCII\x00é中😀�", "ASCII\x00é中😀�"},
		{"truncated two", "\xc2", "�"},
		{"truncated three first", "\xe1", "�"},
		{"truncated three second", "\xe1\x80", "�"},
		{"truncated four first", "\xf1", "�"},
		{"truncated four second", "\xf1\x80", "�"},
		{"truncated four third", "\xf1\x80\x80", "�"},
		{"overlong two", "\xc0\x80", "��"},
		{"overlong three", "\xe0\x9f\x80", "���"},
		{"overlong four", "\xf0\x8f\x80\x80", "����"},
		{"surrogate", "\xed\xa0\x80", "���"},
		{"above maximum", "\xf4\x90\x80\x80", "����"},
		{"invalid lead", "\xf5\x80\x80\x80", "����"},
		{"continuation", "\x80", "�"},
		{"invalid second", "\xc2!", "�!"},
		{"invalid third", "\xe1\x80!", "�!"},
		{"invalid fourth", "\xf1\x80\x80!", "�!"},
		{"invalid before truncation", "\xe0\x80", "��"},
		{"valid prefix", "aé😀\xe1\x80!", "aé😀�!"},
		{"incomplete after prefix", "aé😀\xf1\x80", "aé😀�"},
		{"consecutive errors", "ok\xff\x80", "ok��"},
		{"mixed errors", "\xff\xe1\x80!\xf1\x80", "��!�"},
		{"valid boundaries", "\xc2\x80\xdf\xbf\xe0\xa0\x80\xed\x9f\xbf\xee\x80\x80\xef\xbf\xbf\xf0\x90\x80\x80\xf4\x8f\xbf\xbf", "\u0080\u07ff\u0800\ud7ff\ue000\uffff\U00010000\U0010ffff"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := contract.LossyUTF8([]byte(tc.input)); got != tc.want {
				t.Fatalf("LossyUTF8(%x) = %q; want %q", tc.input, got, tc.want)
			}
		})
	}
}
