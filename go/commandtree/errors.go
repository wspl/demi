package commandtree

import "fmt"

// A DeclarationError says that a declaration breaks one of the tree's rules.
type DeclarationError struct {
	Message string
}

func (e *DeclarationError) Error() string {
	return e.Message
}

func declarationf(format string, args ...any) error {
	return &DeclarationError{Message: fmt.Sprintf(format, args...)}
}

// A UsageError says that a command line does not fit the selected command. Its
// text is what the caller reads.
type UsageError struct {
	Message string
}

func (e *UsageError) Error() string {
	return e.Message
}

func usagef(format string, args ...any) error {
	return &UsageError{Message: fmt.Sprintf(format, args...)}
}
