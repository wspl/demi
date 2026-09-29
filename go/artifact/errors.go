package artifact

import (
	"errors"
	"fmt"
)

// ErrDigest means the bytes do not match their declared SHA-256.
var ErrDigest = errors.New("the artifact does not match its declared SHA-256")

// A DownloadError means the request failed. Its message leaves out the URL,
// which may carry a signature.
type DownloadError struct {
	Message string
}

func (e *DownloadError) Error() string { return e.Message }

// A RejectedError means the server answered with a status other than success.
// Redirects are not followed, so a redirect is one.
type RejectedError struct {
	Status int
}

func (e *RejectedError) Error() string {
	return fmt.Sprintf("the server answered %d", e.Status)
}

// A TooLargeError means the artifact has more bytes than declared.
type TooLargeError struct {
	Declared uint64
}

func (e *TooLargeError) Error() string {
	return fmt.Sprintf("the artifact has more than its declared %d bytes", e.Declared)
}

// A SizeError means the artifact has another number of bytes than declared.
type SizeError struct {
	Declared uint64
	Actual   uint64
}

func (e *SizeError) Error() string {
	return fmt.Sprintf("the artifact has %d bytes, not the declared %d", e.Actual, e.Declared)
}

// An ArchiveError means the archive is not what its installation needs, such as
// one that does not extract or lacks its executable.
type ArchiveError struct {
	Reason string
}

func (e *ArchiveError) Error() string { return "the archive " + e.Reason }

// An InstallationError means an installation in place is not the one its
// receipt and archive name.
type InstallationError struct {
	Directory string
	Reason    string
}

func (e *InstallationError) Error() string {
	return fmt.Sprintf("the installation at %s %s", e.Directory, e.Reason)
}

// A ConflictError means a release already at its directory differs from the one
// being published: a published release is immutable.
type ConflictError struct {
	Path string
}

func (e *ConflictError) Error() string {
	return e.Path + " is already published with other contents"
}
