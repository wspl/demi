package cmdproto

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"runtime"
	"slices"

	"github.com/gowebpki/jcs"
	"github.com/wspl/demi/internal/contract"
)

// Version is the native command wire major version.
const Version = 1

// TargetTriple identifies a supported native executable platform.
// +demi:check validateTargetTriple
// +demi:root
type TargetTriple string

// Targets lists the targets a published release carries, in publication order.
// Callers must treat this shared catalog as read-only.
var Targets = []string{
	"aarch64-apple-darwin",
	"x86_64-apple-darwin",
	"aarch64-unknown-linux-musl",
	"x86_64-unknown-linux-musl",
	"aarch64-pc-windows-msvc",
	"x86_64-pc-windows-msvc",
}

// One target's executable: its SHA-256 and size.
type PackageArtifact struct {
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:range min=1 max=9007199254740991
	Size uint64 `json:"size"`
}

// One target's archive of a resource: its SHA-256 and size, and its
// entry, the file the program uses, as a relative path inside the
// archive with `/` between its components.
// +demi:check validateResourceArtifact
type ResourceArtifact struct {
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:range min=1 max=9007199254740991
	Size  uint64 `json:"size"`
	Entry string `json:"entry"`
}

// Archive returns the bytes to download and verify.
func (r ResourceArtifact) Archive() PackageArtifact {
	return PackageArtifact{SHA256: r.SHA256, Size: r.Size}
}

// What a command program needs on the Host beside its executable, such as
// `demi.browser`'s Chrome (`native-runtime.md` § Bind an exact package):
// its title for the user and the archive of each target that has one.
// +demi:check validatePackageResource
type PackageResource struct {
	// +demi:length chars min=1 max=200
	Title   string                      `json:"title"`
	Targets map[string]ResourceArtifact `json:"targets"`
}

// A command package release: its identity, the operations it serves,
// the artifact of each target it carries and the resources its program
// needs. Publication requires every target; a development release may
// carry fewer.
// +demi:check validatePackageDescriptor
// +demi:root
// +demi:msgpack
type PackageDescriptor struct {
	// +demi:pattern ^[a-z0-9]+([.-][a-z0-9]+)+$
	ID string `json:"id"`
	// +demi:length chars min=1
	Version string `json:"version"`
	// +demi:range min=1 max=1
	ProtocolVersion uint64 `json:"protocolVersion"`
	// +demi:length min=1
	Operations []string                   `json:"operations"`
	Targets    map[string]PackageArtifact `json:"targets"`
	Resources  map[string]PackageResource `json:"resources,omitempty"`
}

// Digest returns the descriptor's RFC 8785 identity.
func (p PackageDescriptor) Digest() (string, error) {
	return CanonicalDigest(p)
}

// Carries finds a pinned executable or resource archive for a target and hash.
func (p PackageDescriptor) Carries(target TargetTriple, digest string) (PackageArtifact, bool) {
	if a, ok := p.Targets[string(target)]; ok && a.SHA256 == digest {
		return a, true
	}
	for _, r := range p.Resources {
		if a, ok := r.Targets[string(target)]; ok && a.SHA256 == digest {
			return a.Archive(), true
		}
	}
	return PackageArtifact{}, false
}

// Serves reports whether the service provides the declared catalog.
func (p PackageDescriptor) Serves(info ServiceInfo) bool {
	if p.ProtocolVersion != info.ProtocolVersion {
		return false
	}
	for _, op := range p.Operations {
		if !slices.Contains(info.Operations, op) {
			return false
		}
	}
	for _, op := range info.Operations {
		if !slices.Contains(p.Operations, op) {
			return false
		}
	}
	return true
}

// Where a runner fetches an artifact.
// +demi:union untagged
// +demi:root
// +demi:msgpack
//
//sumtype:decl
type ArtifactLocation interface{ artifactLocation() }

// A URL the runner downloads an artifact from, valid until `expires_at`
// (milliseconds since the Unix epoch) when set. It is an HTTP or HTTPS URL
// without credentials: object storage answers with a signed HTTPS URL, and a
// development store with a URL on the backend itself. The scheme cannot
// change what runs, because the runner checks the download against the size
// and SHA-256 its pinned descriptor declares (`native-runtime.md` § Install
// the selected package).
// +demi:variant
// +demi:check validateArtifactURL
type ArtifactURL struct {
	URL       string `json:"url"`
	ExpiresAt *int64 `json:"expiresAt,omitempty"`
}

func (*ArtifactURL) artifactLocation() {}

// A path on the runner's machine that holds an artifact.
// +demi:variant
type ArtifactPath struct {
	// +demi:length chars min=1
	Path string `json:"path"`
}

func (*ArtifactPath) artifactLocation() {}

// What a resident service answers on its info path: the protocol it speaks
// and the operations it serves, each once.
// +demi:check validateServiceInfo
// +demi:root
type ServiceInfo struct {
	// +demi:range min=1 max=1
	ProtocolVersion uint64 `json:"protocolVersion"`
	// +demi:length min=1
	Operations []string `json:"operations"`
}

// CanonicalDigest hashes a JSON value using RFC 8785, including its binary64 number model.
func CanonicalDigest(value any) (string, error) {
	// This is an intermediate validation representation, never wire output.
	// EncodeJSON normalizes raw unpaired surrogates before they can be checked;
	// preserve their spelling here so malformed opaque JSON is still refused.
	// JCS below owns all final escaping, ordering and number formatting.
	var data bytes.Buffer
	if err := json.NewEncoder(&data).Encode(value); err != nil {
		return "", fmt.Errorf("encode canonical value: %w", err)
	}
	if err := contract.CheckJSON(data.Bytes()); err != nil {
		return "", fmt.Errorf("canonical value: %w", err)
	}
	canonical, err := jcs.Transform(data.Bytes())
	if err != nil {
		return "", fmt.Errorf("canonicalize value: %w", err)
	}
	return fmt.Sprintf("%x", sha256.Sum256(canonical)), nil
}

// TargetArtifact selects a package executable for the requested platform.
func (p PackageDescriptor) TargetArtifact(target TargetTriple) (PackageArtifact, error) {
	if a, ok := p.Targets[string(target)]; ok {
		return a, nil
	}
	return PackageArtifact{}, fmt.Errorf("%w: %s", ErrMissingTarget, target)
}

// HostTarget returns the native release triple for this build.
func HostTarget() (TargetTriple, error) {
	arch := map[string]string{"amd64": "x86_64", "arm64": "aarch64"}[runtime.GOARCH]
	system := map[string]string{
		"darwin":  "apple-darwin",
		"linux":   "unknown-linux-musl",
		"windows": "pc-windows-msvc",
	}[runtime.GOOS]
	if arch == "" || system == "" {
		return "", fmt.Errorf("unsupported platform %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return TargetTriple(arch + "-" + system), nil
}
