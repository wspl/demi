package runnerwire

import "github.com/wspl/demi/internal/commandwire"

//go:generate go run github.com/wspl/demi/tools/contractgen

// A runner release. Publication requires every target; a development
// release may carry fewer.
// +demi:root
// +demi:check validateRelease
type RunnerRelease struct {
	// The release's identity: the SHA-256 of its versions and targets.
	// +demi:pattern ^[0-9a-f]{64}$
	Release string `json:"release"`
	// +demi:range min=24 max=24
	Wire uint32 `json:"wire"`
	// +demi:range min=1 max=1
	CommandProtocol uint64                                 `json:"commandProtocol"`
	Targets         map[string]commandwire.PackageArtifact `json:"targets"`
}

// validateRelease checks the executable catalog shared with command packages.
func validateRelease(release RunnerRelease) error {
	return commandwire.TargetArtifacts(release.Targets)
}
