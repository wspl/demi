package builtincommands

import (
	"errors"
	"fmt"
	"strconv"
	"syscall"
	"unicode"
	"unicode/utf8"
)

// A failure is a reason the agent reads. Its text is what the Rust
// implementation says, in its words, so it is not written as Go error strings
// are: it begins with a capital letter where the Rust's does.
type failure struct {
	text string
}

func (f *failure) Error() string { return f.text }

// fail returns a failure with a formatted reason.
func fail(format string, args ...any) error {
	return &failure{text: fmt.Sprintf(format, args...)}
}

// errCancelled is the failure of work that stopped because its invocation was
// cancelled. It is not a reason the agent reads: the invocation answers
// nothing. Inside a rollback's report it is one, in these words.
var errCancelled = &failure{text: "Command cancelled"}

// An ioFailure is a failure of the operating system, worded as Rust words an I/O
// error: the system's message with its first letter capitalized, and its
// number.
type ioFailure struct {
	err error
}

// system returns err, an error of the operating system, as the failure the agent
// reads, or nil for nil.
func system(err error) error {
	if err == nil {
		return nil
	}
	return &ioFailure{err: err}
}

func (f *ioFailure) Error() string {
	var errno syscall.Errno
	if errors.As(f.err, &errno) {
		message := errno.Error()
		first, size := utf8.DecodeRuneInString(message)
		return string(unicode.ToUpper(first)) + message[size:] + " (os error " + strconv.Itoa(int(errno)) + ")"
	}
	return f.err.Error()
}

// Unwrap returns the error of the system, which a caller may look for.
func (f *ioFailure) Unwrap() error { return f.err }

// errNotUTF8 is what reading a file that is not text answers.
var errNotUTF8 = &failure{text: "stream did not contain valid UTF-8"}

// errNoParent is the failure of a file that has no directory to be in.
var errNoParent = &failure{text: "File has no parent directory"}
