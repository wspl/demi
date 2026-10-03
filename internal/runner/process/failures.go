package process

// operationError preserves the runner's diagnostic while retaining a typed
// cause for callers checking cancellation or a closed process input.
type operationError struct {
	message string
	cause   error
}

func (e *operationError) Error() string { return e.message }
func (e *operationError) Unwrap() error { return e.cause }
