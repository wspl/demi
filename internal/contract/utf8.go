package contract

import (
	"fmt"
	"unicode/utf8"
)

// UTF8Error describes the first malformed sequence, matching Rust's Utf8Error.
type UTF8Error struct {
	// ValidUpTo is the byte index of the first malformed sequence.
	ValidUpTo int
	// ErrorLen is the malformed sequence's width, or zero for an incomplete suffix.
	ErrorLen int
}

func (e *UTF8Error) Error() string {
	if e.ErrorLen == 0 {
		return fmt.Sprintf("incomplete utf-8 byte sequence from index %d", e.ValidUpTo)
	}
	return fmt.Sprintf("invalid utf-8 sequence of %d bytes from index %d", e.ErrorLen, e.ValidUpTo)
}

// CheckUTF8 returns nil for valid UTF-8, otherwise a *UTF8Error whose text
// matches Rust's std::str::Utf8Error Display for the same bytes.
func CheckUTF8(data []byte) error {
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
			if next < 0x80 || next > 0xbf || offset == 1 && (first == 0xe0 && next < 0xa0 || first == 0xed && next >= 0xa0 || first == 0xf0 && next < 0x90 || first == 0xf4 && next >= 0x90) {
				return &UTF8Error{ValidUpTo: index, ErrorLen: offset}
			}
		}
		return &UTF8Error{ValidUpTo: index, ErrorLen: 1}
	}
	return nil
}
