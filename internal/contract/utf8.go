package contract

import (
	"fmt"
	"strings"
	"unicode/utf8"
)

// UTF8Error describes the first malformed sequence, matching Rust's Utf8Error.
type UTF8Error struct {
	// ValidUpTo is the byte index of the first malformed sequence.
	ValidUpTo int
	// ErrorLen is the malformed sequence's width, or zero for an incomplete suffix.
	ErrorLen int
}

// Error describes the malformed UTF-8 sequence using Rust diagnostics.
func (e *UTF8Error) Error() string {
	if e.ErrorLen == 0 {
		return fmt.Sprintf("incomplete utf-8 byte sequence from index %d", e.ValidUpTo)
	}
	return fmt.Sprintf("invalid utf-8 sequence of %d bytes from index %d", e.ErrorLen, e.ValidUpTo)
}

// CheckUTF8 returns nil for valid UTF-8, otherwise a *UTF8Error whose text
// matches Rust's std::str::Utf8Error Display for the same bytes.
func CheckUTF8(data []byte) error {
	if invalid := checkUTF8(data); invalid != nil {
		return invalid
	}
	return nil
}

// checkUTF8 identifies the first maximal invalid subpart for both contract
// validation and Rust-compatible lossy decoding.
func checkUTF8(data []byte) *UTF8Error {
	for index := 0; index < len(data); {
		r, size := utf8.DecodeRune(data[index:])
		if r != utf8.RuneError || size != 1 {
			index += size
			continue
		}
		first := data[index]
		width := 1
		switch {
		case first >= 0xc2 && first <= 0xdf:
			width = 2
		case first >= 0xe0 && first <= 0xef:
			width = 3
		case first >= 0xf0 && first <= 0xf4:
			width = 4
		}
		for offset := 1; offset < width; offset++ {
			if index+offset >= len(data) {
				return &UTF8Error{ValidUpTo: index}
			}
			next := data[index+offset]
			if next < 0x80 || next > 0xbf ||
				offset == 1 &&
					(first == 0xe0 && next < 0xa0 || first == 0xed && next >= 0xa0 ||
						first == 0xf0 && next < 0x90 || first == 0xf4 && next >= 0x90) {
				return &UTF8Error{ValidUpTo: index, ErrorLen: offset}
			}
		}
		return &UTF8Error{ValidUpTo: index, ErrorLen: 1}
	}
	return nil
}

// LossyUTF8 decodes data like Rust's String::from_utf8_lossy, replacing each
// maximal invalid UTF-8 subpart with one U+FFFD.
func LossyUTF8(data []byte) string {
	var text strings.Builder
	for {
		invalid := checkUTF8(data)
		if invalid == nil {
			text.Write(data)
			return text.String()
		}
		text.Write(data[:invalid.ValidUpTo])
		text.WriteRune(utf8.RuneError)
		if invalid.ErrorLen == 0 {
			return text.String()
		}
		data = data[invalid.ValidUpTo+invalid.ErrorLen:]
	}
}
