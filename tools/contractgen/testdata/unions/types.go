// Package unions exercises reply and Go-only untagged boundaries.
package unions

import "encoding/json"

//go:generate go run ../..

// +demi:root direction=receive output=plugin-unions
// +demi:union tag=ok
// +demi:msgpack
type EnsureReply interface{ ensureReply() }

// +demi:root direction=receive output=plugin-unions
// +demi:union tag=ok
// +demi:msgpack
type StatusReply interface{ statusReply() }

// +demi:variant true
type Ensured struct {
	Version string `json:"version"`
	Path    string `json:"path"`
}

func (*Ensured) ensureReply() {}

// +demi:variant true
type Status struct {
	Platform  string      `json:"platform"`
	Installed []Installed `json:"installed"`
}

func (*Status) statusReply() {}

type Installed struct {
	Version string `json:"version"`
	Path    string `json:"path"`
}

// +demi:enum invalid_release unsupported_platform install_failed
type ErrorCode string

// +demi:variant false
type Failure struct {
	Code    ErrorCode `json:"code"`
	Message string    `json:"message"`
}

func (*Failure) ensureReply() {}
func (*Failure) statusReply() {}

// +demi:root direction=receive
// +demi:union untagged
type Node interface{ node() }

// +demi:variant
type Group struct {
	Name        string `json:"name"`
	Summary     string `json:"summary"`
	Subcommands []Node `json:"subcommands"`
}

func (*Group) node() {}

// +demi:variant
type Leaf struct {
	Name          string           `json:"name"`
	Summary       string           `json:"summary"`
	SuccessOutput *string          `json:"successOutput,omitempty"`
	FailureOutput *string          `json:"failureOutput,omitempty"`
	RunningHint   *string          `json:"runningHint,omitempty"`
	Input         *json.RawMessage `json:"input,omitempty"`
	Positionals   *[]string        `json:"positionals,omitempty"`
	StdinField    *string          `json:"stdinField,omitempty"`
	RestField     *string          `json:"restField,omitempty"`
	Output        *LeafOutput      `json:"output,omitempty"`
	// +demi:enum rpc native
	Kind    string   `json:"kind"`
	Binding *Binding `json:"binding,omitempty"`
}

func (*Leaf) node() {}

type LeafOutput struct {
	JSON *json.RawMessage `json:"json,omitempty"`
}
type Binding struct {
	Package        string `json:"package"`
	Operation      string `json:"operation"`
	DescriptorHash string `json:"descriptorHash"`
}

// +demi:root direction=receive
// +demi:union untagged
// +demi:msgpack
type ArtifactLocation interface{ location() }

// +demi:variant
type ArtifactURL struct {
	URL       string `json:"url"`
	ExpiresAt *int64 `json:"expiresAt,omitempty"`
}

func (*ArtifactURL) location() {}

// +demi:variant
type ArtifactPath struct {
	// +demi:length chars min=1
	Path string `json:"path"`
}

func (*ArtifactPath) location() {}

// +demi:root direction=receive
// +demi:union untagged
// +demi:msgpack
type ArtifactOwner interface{ owner() }

// +demi:variant
type JobArtifactOwner struct {
	JobID string `json:"jobId"`
	// +demi:pattern ^[a-f0-9]{64}$
	ManifestHash string `json:"manifestHash"`
}

func (*JobArtifactOwner) owner() {}

// +demi:variant
type StreamArtifactOwner struct {
	StreamID string `json:"streamId"`
}

func (*StreamArtifactOwner) owner() {}

// +demi:root direction=receive
// +demi:union tag=type
// +demi:msgpack
type Frame interface{ frame() }

// +demi:variant Frame artifact_location
type LocationFrame struct {
	ID       string            `json:"id"`
	Location *ArtifactLocation `json:"location,omitempty"`
	Error    *string           `json:"error,omitempty"`
}

// +demi:variant Frame artifact_resolve
type ResolveFrame struct {
	ID     string        `json:"id"`
	Owner  ArtifactOwner `json:"owner"`
	SHA256 string        `json:"sha256"`
	Target string        `json:"target"`
}

// +demi:root direction=receive output=plugin-unions
// +demi:msgpack
type Empty struct {
	Resources map[string]string `json:"resources,omitempty"`
	Items     []string          `json:"items,omitempty"`
	Bytes     Bytes             `json:"bytes,omitempty"`
}

// +demi:base64
type Bytes []byte

// Runtime-only declarations, including unsupported wire shapes and markers,
// must not acquire codecs or prevent boundary generation.
// +demi:check missingCheck
type CompiledSchema struct {
	Match func(any) bool
	Cache chan int
}

// +demi:root direction=receive
// +demi:union untagged
// +demi:msgpack
type Ordered interface{ ordered() }

// +demi:variant
type ZFirst struct {
	Value string `json:"value"`
}

func (*ZFirst) ordered() {}

// +demi:variant
type ASecond struct {
	Value string `json:"value"`
}

func (*ASecond) ordered() {}

// +demi:root direction=receive output=plugin-unions
type WireAPI struct {
	ID      BlockID           `json:"id"`
	Failure HTTPFailureRecord `json:"failure"`
}

// +demi:root direction=receive output=plugin-unions
type BlockID string

type HTTPFailureRecord struct {
	Code string `json:"code"`
}
