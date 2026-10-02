//go:build linux

package storage

import "errors"

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

// CorruptRecordError reports a generation record that cannot be decoded.
type CorruptRecordError struct {
	// Path is the record's filename.
	Path string
	// Source is the decoding failure.
	Source error
}

// Error describes the invalid generation record.
func (e *CorruptRecordError) Error() string { panic("not written: m-storage") }

// Unwrap returns the decoding failure.
func (e *CorruptRecordError) Unwrap() error { panic("not written: m-storage") }

// NotExt4Error reports an invalid ext4 superblock.
type NotExt4Error struct {
	// Path is the image filename.
	Path string
}

// Error describes the invalid image.
func (e *NotExt4Error) Error() string { panic("not written: m-storage") }

// GrowthCapabilityError reports a resize failure when the manager's bounding
// capability set lacks CAP_SYS_RESOURCE.
type GrowthCapabilityError struct {
	// Source is the resize tool failure.
	Source error
}

// Error names the missing capability.
func (e *GrowthCapabilityError) Error() string { panic("not written: m-storage") }

// Unwrap returns the resize tool failure.
func (e *GrowthCapabilityError) Unwrap() error { panic("not written: m-storage") }

var (
	// ErrUnsafeEntry reports an unsafe archive member.
	//nolint:staticcheck // Preserve the Rust user-facing diagnostic verbatim.
	ErrUnsafeEntry = errors.New("Cloud archive contains an unsafe path or entry type")
	// ErrUnsafeHardlink reports an unsafe hard-link target.
	//nolint:staticcheck // Preserve the Rust user-facing diagnostic verbatim.
	ErrUnsafeHardlink = errors.New("Cloud archive contains an unsafe hardlink")
	// ErrArchitecture reports an incompatible base architecture.
	//nolint:staticcheck // Preserve the Rust user-facing diagnostic verbatim.
	ErrArchitecture = errors.New("Cloud image architecture differs from execution host")
	// ErrPinnedManifest reports different manifest bytes at the pinned version.
	//nolint:staticcheck // Preserve the Rust user-facing diagnostic verbatim.
	ErrPinnedManifest = errors.New("Pinned Cloud image manifest differs")
)

// MissingExecutableError reports a required executable absent from the manifest.
type MissingExecutableError struct {
	// Path is the executable path.
	Path string
}

// Error describes the executable failure.
func (e *MissingExecutableError) Error() string { panic("not written: m-storage") }

// ExecutablePathError reports an executable path that escapes the extracted root.
type ExecutablePathError struct {
	// Path is the executable path.
	Path string
}

// Error describes the executable failure.
func (e *ExecutablePathError) Error() string { panic("not written: m-storage") }

// ExecutableIntegrityError reports an executable with the wrong kind, size, or digest.
type ExecutableIntegrityError struct {
	// Path is the executable path.
	Path string
}

// Error describes the executable failure.
func (e *ExecutableIntegrityError) Error() string { panic("not written: m-storage") }

// ArchiveIntegrityError reports a base archive whose size or digest differs
// from the release manifest.
type ArchiveIntegrityError struct {
	// Source is the artifact verification failure.
	Source error
}

// Error describes the archive integrity mismatch.
func (e *ArchiveIntegrityError) Error() string { panic("not written: m-storage") }

// Unwrap returns the artifact verification failure.
func (e *ArchiveIntegrityError) Unwrap() error { panic("not written: m-storage") }
