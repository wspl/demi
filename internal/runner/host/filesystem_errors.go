package host

// filesystemError preserves the Host's user-facing words and an OS error
// identity for protocol classification, without adding the syscall's wording.
type filesystemError struct {
	message string
	cause   error
}

func (e *filesystemError) Error() string {
	return e.message
}

func (e *filesystemError) Unwrap() error {
	return e.cause
}
