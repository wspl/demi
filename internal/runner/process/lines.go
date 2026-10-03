package process

import (
	"strings"

	"golang.org/x/text/encoding/unicode"
)

// LineBytes is the byte limit after which a log line continues on the next line.
const LineBytes = 4096

// LineSplitter cuts arbitrary byte chunks into nonempty log lines at newlines
// or LineBytes. Its zero value is ready to use.
type LineSplitter struct{ pending []byte }

// Push returns the lines chunk completes, retaining an unfinished line.
func (s *LineSplitter) Push(chunk []byte) []string {
	var lines []string
	for _, b := range chunk {
		if b != '\n' {
			s.pending = append(s.pending, b)
		}
		if b == '\n' || len(s.pending) == LineBytes {
			if line, ok := s.Finish(); ok {
				lines = append(lines, line)
			}
		}
	}
	return lines
}

// Finish returns the final unterminated nonempty line, and false when there is none.
func (s *LineSplitter) Finish() (string, bool) {
	text := strings.TrimRight(streamText(s.pending), "\r")
	s.pending = s.pending[:0]
	if text == "" {
		return "", false
	}
	return text, true
}

// streamText replaces malformed stream bytes by Unicode replacement characters,
// including a single replacement for a truncated character at a tail boundary.
func streamText(b []byte) string {
	// The UTF-8 decoder replaces malformed input and cannot reject it.
	text, _ := unicode.UTF8.NewDecoder().String(string(b))
	return text
}
