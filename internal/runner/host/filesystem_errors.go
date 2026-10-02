package host

// filesystemFailure preserves the Host's user-facing words and an OS error
// identity for protocol classification, without adding the syscall's wording.
type filesystemFailure struct {
	message string
	cause   error
}

func (e *filesystemFailure) Error() string { return e.message }
func (e *filesystemFailure) Unwrap() error { return e.cause }
