package machinemanager

import (
	"errors"
	"strings"
)

// ErrorChain renders an operation error and its causes for the manager's log.
// Wire replies carry only the operation's own Error message.
func ErrorChain(err error) string {
	if err == nil {
		return ""
	}
	if parts, ok := err.(interface{ Unwrap() []error }); ok {
		causes := parts.Unwrap()
		text := make([]string, 0, len(causes))
		for _, cause := range causes {
			if cause != nil {
				text = append(text, ErrorChain(cause))
			}
		}
		return strings.Join(text, "; ")
	}
	text := err.Error()
	if cause := errors.Unwrap(err); cause != nil {
		text += ": " + ErrorChain(cause)
	}
	return text
}
