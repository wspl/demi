package zodrt

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
)

// Error is a value that does not satisfy its schema: where in the value, and
// what is wrong there. It is the Go form of a Zod issue.
type Error struct {
	// Path names the object keys and array indexes from the root to the value.
	Path    []string
	Message string
}

func (e *Error) Error() string {
	if len(e.Path) == 0 {
		return e.Message
	}
	return strings.Join(e.Path, ".") + ": " + e.Message
}

// Invalid reports a value that breaks its schema.
func Invalid(format string, args ...any) error {
	return &Error{Message: fmt.Sprintf(format, args...)}
}

// At places an error under an object key. A nil error stays nil.
func At(key string, err error) error {
	if err == nil {
		return nil
	}
	var issue *Error
	if errors.As(err, &issue) {
		return &Error{Path: append([]string{key}, issue.Path...), Message: issue.Message}
	}
	return &Error{Path: []string{key}, Message: err.Error()}
}

// AtIndex places an error under an array index. A nil error stays nil.
func AtIndex(index int, err error) error {
	return At(strconv.Itoa(index), err)
}

// NoMatch reports a union none of whose options accepted the value. With
// exactly one candidate, its own error is the more useful report.
func NoMatch(errs []error) error {
	if len(errs) == 1 {
		return errs[0]
	}
	return Invalid("no union option matches the value")
}
