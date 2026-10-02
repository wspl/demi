package commandwire

import (
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
// +demi:enum aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl aarch64-pc-windows-msvc x86_64-pc-windows-msvc
type TargetTriple string

// PackageArtifact identifies executable bytes.
type PackageArtifact struct {
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:range min=1 max=9007199254740991
	Size uint64 `json:"size"`
}

// ResourceArtifact identifies an archive and the entry its consumer starts.
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

// PackageResource describes a program's runtime dependency.
// +demi:check validatePackageResource
type PackageResource struct {
	// +demi:length chars min=1 max=200
	Title   string                      `json:"title"`
	Targets map[string]ResourceArtifact `json:"targets"`
}

// PackageDescriptor pins one immutable command release.
// +demi:check validatePackageDescriptor
type PackageDescriptor struct {
	// +demi:pattern ^[a-z0-9]+([.-][a-z0-9]+)+$
	ID string `json:"id"`
	// +demi:length chars min=1
	Version string `json:"version"`
	// +demi:range min=1 max=1
	ProtocolVersion uint64 `json:"protocolVersion"`
	// +demi:length min=1
	Operations []string                    `json:"operations"`
	Targets    map[string]PackageArtifact  `json:"targets"`
	Resources  *map[string]PackageResource `json:"resources,omitempty"`
}

// Digest returns the descriptor's RFC 8785 identity.
func (p PackageDescriptor) Digest() (string, error) { return CanonicalDigest(p) }

// Carries finds a pinned executable or resource archive for a target and hash.
func (p PackageDescriptor) Carries(target TargetTriple, digest string) (PackageArtifact, bool) {
	if a, ok := p.Targets[string(target)]; ok && a.SHA256 == digest {
		return a, true
	}
	if p.Resources != nil {
		for _, r := range *p.Resources {
			if a, ok := r.Targets[string(target)]; ok && a.SHA256 == digest {
				return a.Archive(), true
			}
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

// ArtifactLocation tells the runner where to fetch artifact bytes.
// +demi:union tag=kind
//
//sumtype:decl
type ArtifactLocation interface{ artifactLocation() }

// ArtifactURL is an HTTP(S) download without credentials.
// +demi:variant ArtifactLocation url
// +demi:check validateArtifactURL
type ArtifactURL struct {
	URL       string `json:"url"`
	ExpiresAt *int64 `json:"expiresAt,omitempty"`
}

func (*ArtifactURL) artifactLocation() {}

// ArtifactPath is a path on the runner's machine.
// +demi:variant ArtifactLocation path
type ArtifactPath struct {
	// +demi:length chars min=1
	Path string `json:"path"`
}

func (*ArtifactPath) artifactLocation() {}

// ServiceInfo advertises a resident service's protocol and operations.
// +demi:check validateServiceInfo
type ServiceInfo struct {
	// +demi:range min=1 max=1
	ProtocolVersion uint64 `json:"protocolVersion"`
	// +demi:length min=1
	Operations []string `json:"operations"`
}

// CanonicalDigest hashes a JSON value using RFC 8785, including its binary64 number model.
func CanonicalDigest(value any) (string, error) {
	data, err := json.Marshal(value)
	if err != nil {
		return "", fmt.Errorf("encode canonical value: %w", err)
	}
	if err := contract.CheckJSON(data); err != nil {
		return "", fmt.Errorf("canonical value: %w", err)
	}
	canonical, err := jcs.Transform(data)
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
	os := map[string]string{"darwin": "apple-darwin", "linux": "unknown-linux-musl", "windows": "pc-windows-msvc"}[runtime.GOOS]
	if arch == "" || os == "" {
		return "", fmt.Errorf("unsupported platform %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return TargetTriple(arch + "-" + os), nil
}
