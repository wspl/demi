package claudecodeop

//go:generate go run github.com/wspl/demi/tools/contractgen

// Package is the package's id in the native catalog.
const Package = "demi.claude-code"

// The package's operations.
// +demi:root direction=receive
// +demi:id
// +demi:enum claude-code.ensure claude-code.status
type Operation string

// Operation names in the native descriptor.
const (
	// `claude-code.ensure`: installs the version a release record names.
	OperationEnsure Operation = "claude-code.ensure"
	// `claude-code.status`: lists the installed versions.
	OperationStatus Operation = "claude-code.status"
)

// `version` as a SemVer version when it is one without build metadata, such
// as `2.1.3` or `2.1.3-beta.1`. A version names its installation directory,
// so `+build` is refused.
// +demi:root direction=receive
// +demi:id
// +demi:check validateVersion
type Version string

// What `claude-code.ensure` reads from stdin: one CLI version and its official
// executable for each platform key. Every entry is checked, not only this
// machine's.
// +demi:root direction=receive
// +demi:strict
type Release struct {
	Version   Version             `json:"version"`
	Platforms map[string]Artifact `json:"platforms"`
}

// Where one platform's executable is, its byte size and its SHA-256 in
// lowercase hex.
// +demi:check validateArtifact
type Artifact struct {
	URL string `json:"url"`
	// +demi:range min=1
	Size uint64 `json:"size"`
	// +demi:pattern ^[a-f0-9]{64}$
	SHA256 string `json:"sha256"`
}

// One usable executable.
// +demi:root direction=receive
// +demi:strict
type Installed struct {
	Version string `json:"version"`
	Path    string `json:"path"`
}

// What `claude-code.status` answers: this machine's platform key and the
// versions the runner has, the newest install first.
// +demi:root direction=receive
// +demi:strict
type Status struct {
	Platform  string      `json:"platform"`
	Installed []Installed `json:"installed"`
}

// Why an operation failed; a cancelled invocation writes no document.
// +demi:enum invalid_release unsupported_platform install_failed
type ErrorCode string

// Failure codes returned by installation operations.
const (
	InvalidRelease      ErrorCode = "invalid_release"
	UnsupportedPlatform ErrorCode = "unsupported_platform"
	InstallFailed       ErrorCode = "install_failed"
)

// Failure describes an operation that could not complete.
// +demi:root direction=receive
// +demi:strict
type Failure struct {
	Code    ErrorCode `json:"code"`
	Message string    `json:"message"`
}

// The document an operation writes: `{"ok": true, ...answer}`, or
// `{"ok": false, "code", "message"}`.
// +demi:root direction=receive
// +demi:union tag=ok
//
//sumtype:decl
type EnsureReply interface{ ensureReply() }

// The document an operation writes: `{"ok": true, ...answer}`, or
// `{"ok": false, "code", "message"}`.
// +demi:root direction=receive
// +demi:union tag=ok
//
//sumtype:decl
type StatusReply interface{ statusReply() }

// Ensured is the successful ensure reply.
// +demi:variant true
type Ensured struct{ Installed }

func (*Ensured) ensureReply() {}

// StatusDone is the successful status reply.
// +demi:variant true
type StatusDone struct{ Status }

func (*StatusDone) statusReply() {}

// Failed is the failure reply shared by both operations.
// +demi:variant false
type Failed struct{ Failure }

func (*Failed) ensureReply() {}
func (*Failed) statusReply() {}

// Operations returns every operation, in the order the descriptor lists them.
func Operations() [2]Operation {
	return [2]Operation{OperationEnsure, OperationStatus}
}
