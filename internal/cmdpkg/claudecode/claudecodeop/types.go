package claudecodeop

//go:generate go run ../../../../tools/contractgen

// Package is the native catalog identifier.
const Package = "demi.claude-code"

// Operation names an operation provided by this command package.
// +demi:enum claude-code.ensure claude-code.status
type Operation string

// Operation names in the native descriptor.
const (
	OperationEnsure Operation = "claude-code.ensure"
	OperationStatus Operation = "claude-code.status"
)

// Version names an installation directory: three numbers and an optional
// prerelease, without a v prefix or build metadata.
// +demi:check validateVersion
type Version string

// Release is the input to claude-code.ensure. Every platform is validated.
// +demi:strict
type Release struct {
	Version   Version             `json:"version"`
	Platforms map[string]Artifact `json:"platforms"`
}

// Artifact describes one platform's executable.
// +demi:check validateArtifact
type Artifact struct {
	URL string `json:"url"`
	// +demi:range min=1
	Size uint64 `json:"size"`
	// +demi:pattern ^[a-f0-9]{64}$
	SHA256 string `json:"sha256"`
}

// Installed identifies a usable executable on the target machine.
// +demi:strict
type Installed struct {
	Version string `json:"version"`
	Path    string `json:"path"`
}

// Status lists installations in newest-install-first order.
// +demi:strict
type Status struct {
	Platform  string      `json:"platform"`
	Installed []Installed `json:"installed"`
}

// ErrorCode identifies an operation failure.
// +demi:enum invalid_release unsupported_platform install_failed
type ErrorCode string

// Failure codes returned by installation operations.
const (
	InvalidRelease      ErrorCode = "invalid_release"
	UnsupportedPlatform ErrorCode = "unsupported_platform"
	InstallFailed       ErrorCode = "install_failed"
)

// Failure describes an operation that could not complete.
// +demi:strict
type Failure struct {
	Code    ErrorCode `json:"code"`
	Message string    `json:"message"`
}
