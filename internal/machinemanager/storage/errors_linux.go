//go:build linux

package storage

import (
	"errors"
)

// GrowthCapabilityError reports a resize failure when the manager's bounding
// capability set lacks CAP_SYS_RESOURCE.
type GrowthCapabilityError struct {
	// Source is the resize tool failure.
	Source error
}

// Error names the missing capability.
func (e *GrowthCapabilityError) Error() string {
	return "growing a mounted ext4 filesystem needs CAP_SYS_RESOURCE; this host's manager lacks it"
}

// Unwrap returns the resize tool failure.
func (e *GrowthCapabilityError) Unwrap() error {
	return e.Source
}

var (
	// ErrUnsafeEntry reports an unsafe archive member.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrUnsafeEntry = errors.New("Cloud archive contains an unsafe path or entry type")
	// ErrUnsafeHardlink reports an unsafe hard-link target.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrUnsafeHardlink = errors.New("Cloud archive contains an unsafe hardlink")
	// ErrArchitecture reports an incompatible base architecture.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrArchitecture = errors.New("Cloud image architecture differs from execution host")
	// ErrPinnedManifest reports different manifest bytes at the pinned version.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrPinnedManifest = errors.New("Pinned Cloud image manifest differs")
)
