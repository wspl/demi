package process

// operationFailure preserves the runner's diagnostic while retaining a typed
// cause for callers checking cancellation or a closed process input.
type operationFailure struct {
	message string
	cause   error
}

func (e *operationFailure) Error() string { return e.message }
func (e *operationFailure) Unwrap() error { return e.cause }
