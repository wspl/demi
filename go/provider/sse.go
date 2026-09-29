package provider

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"iter"
	"strings"
	"unicode/utf8"
)

// ErrSSEUTF8 distinguishes malformed vendor text from a broken transport.
var ErrSSEUTF8 = errors.New("the event stream is not UTF-8")

// SSEData reads data fields, including a final event without its blank line.
// The caller owns the body; cancelling an HTTP request interrupts its reads.
func SSEData(body io.Reader) iter.Seq2[string, error] {
	return func(yield func(string, error) bool) {
		reader := bufio.NewReader(body)
		var line, data strings.Builder
		first := true
		skipLF := false
		emit := func() bool {
			text := strings.TrimSuffix(data.String(), "\n")
			data.Reset()
			return text == "" || yield(text, nil)
		}
		consume := func() bool {
			text := line.String()
			line.Reset()
			if !utf8.ValidString(text) {
				yield("", ErrSSEUTF8)
				return false
			}
			if first {
				text = strings.TrimPrefix(text, "\ufeff")
				first = false
			}
			if text == "" {
				return emit()
			}
			field, value, _ := strings.Cut(text, ":")
			if field == "data" {
				data.WriteString(strings.TrimPrefix(value, " "))
				data.WriteByte('\n')
			}
			return true
		}
		for {
			b, err := reader.ReadByte()
			if err != nil {
				if err != io.EOF {
					yield("", fmt.Errorf("the event stream broke off: %w", err))
					return
				}
				if line.Len() != 0 && !consume() {
					return
				}
				emit()
				return
			}
			if skipLF {
				skipLF = false
				if b == '\n' {
					continue
				}
			}
			if b == '\r' || b == '\n' {
				if !consume() {
					return
				}
				skipLF = b == '\r'
			} else {
				line.WriteByte(b)
			}
		}
	}
}
