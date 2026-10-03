package sandbox

// revive:disable:exported Contract documentation preserves the Rust product text.

//go:generate go run github.com/wspl/demi/tools/contractgen

// One boot's id, `demi-` and a UUID: it names the runtime directory, the
// runsc container and the cgroup.
// +demi:root
// +demi:id
// +demi:pattern ^demi-[A-Za-z0-9_-]+$
type ID string

// `sandbox.json` in the working pair: written before any resource of the
// boot exists and removed after the last is released, so its presence
// means recovery must fence the boot. It holds no credential.
// +demi:root
type Record struct {
	ID   ID     `json:"id"`
	Slot uint16 `json:"slot"`
}

// The runtime release the manager is built with (`runtime/release.json`).
// +demi:root
type RuntimeRelease struct {
	Upstream           string `json:"upstream"`
	Commit             string `json:"commit"`
	Arm64Version       string `json:"arm64Version"`
	Arm64PatchSHA256   string `json:"arm64PatchSha256"`
	AMD64ArchiveSHA512 string `json:"amd64ArchiveSha512"`
	Bazel              string `json:"bazel"`
	BazelArm64SHA256   string `json:"bazelArm64Sha256"`
}

// A container's state, as `runsc list` reports it.
// +demi:root
// +demi:enum creating created running paused stopped
type Status string

const (
	// Creating means runsc is creating the container.
	Creating Status = "creating"
	// Created means the container is created but has not started.
	Created Status = "created"
	// Running means the container is executing.
	Running Status = "running"
	// Paused means execution is suspended for a checkpoint.
	Paused Status = "paused"
	// Stopped means the container exited.
	Stopped Status = "stopped"
)

// +demi:root
type runtimeListing []runtimeContainer

// +demi:tolerant
type runtimeContainer struct {
	ID     string `json:"id"`
	Status Status `json:"status"`
}

// +demi:root
type namespaceOwner struct {
	DataDir string `json:"dataDir"`
}
