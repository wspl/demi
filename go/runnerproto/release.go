package runnerproto

import (
	"github.com/wspl/demi/go/commandservice"
)

// RunnerRelease names a runner release and each target's executable.
//
//demi:wire
type RunnerRelease struct {
	Release         string                                    `json:"release" check:"func=releaseDigest"`
	Wire            uint32                                    `json:"wire" check:"eq=Version"`
	CommandProtocol uint64                                    `json:"commandProtocol" check:"eq=commandservice.Version"`
	Targets         map[string]commandservice.PackageArtifact `json:"targets" check:"keys(func=commandservice.ValidateTarget),each(func=commandservice.Validate)"`
}

// DecodeRunnerRelease reads and validates a release manifest.
func DecodeRunnerRelease(data []byte) (RunnerRelease, error) { return decode[RunnerRelease](data) }

// Encode writes a validated release manifest.
func (r RunnerRelease) Encode() ([]byte, error) { return encode(r) }

func releaseDigest(value string) error {
	// PackageArtifact owns the digest rule; its nonzero size is unrelated here.
	return commandservice.Validate(commandservice.PackageArtifact{SHA256: value, Size: 1})
}
