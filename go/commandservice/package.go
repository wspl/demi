package commandservice

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"fmt"
	"slices"
)

// maxSafeInteger is the largest integer a JavaScript peer holds exactly.
const maxSafeInteger = 1<<53 - 1

// A PackageArtifact is one target's executable: its SHA-256 and size.
//
//demi:wire
type PackageArtifact struct {
	SHA256 string `json:"sha256" check:"pattern=sha256Hex"`
	Size   uint64 `json:"size" check:"range=1..maxSafeInteger"`
}

// A PackageDescriptor is a native command package release: its identity, the
// operations it serves and the artifact of each target it carries.
// Publication requires every target; a development release may carry fewer.
//
//demi:wire
//demi:msgpack
type PackageDescriptor struct {
	ID              string                           `json:"id" check:"pattern=packageID"`
	Version         string                           `json:"version" check:"chars=1.."`
	ProtocolVersion uint64                           `json:"protocolVersion" check:"eq=Version"`
	Operations      []string                         `json:"operations" check:"items=1..,unique,each(chars=1..)"`
	Targets         map[TargetTriple]PackageArtifact `json:"targets" check:"keys(oneof=aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-musl|x86_64-unknown-linux-musl|aarch64-pc-windows-msvc|x86_64-pc-windows-msvc)"`
}

// Digest returns the descriptor's identity: the SHA-256 of its canonical JSON.
// It refuses a descriptor that breaks the rules of its type.
func (d PackageDescriptor) Digest() (string, error) {
	data, err := Encode(d)
	if err != nil {
		return "", err
	}
	// Encode returned data for this call alone, so it may be rewritten in place.
	canonical := jsontext.Value(data)
	if err := canonical.Canonicalize(); err != nil {
		return "", err
	}
	sum := sha256.Sum256(canonical)
	return hex.EncodeToString(sum[:]), nil
}

// Serves reports whether a service's catalog is the one the descriptor
// declares: the same protocol and the same operations, in any order.
func (d PackageDescriptor) Serves(info ServiceInfo) bool {
	return info.ProtocolVersion == d.ProtocolVersion && slices.Equal(operationSet(d.Operations), operationSet(info.Operations))
}

func operationSet(operations []string) []string {
	return slices.Compact(slices.Sorted(slices.Values(operations)))
}

// Artifact returns the executable the package carries for target, or an error
// that matches [ErrMissingTarget].
func (d PackageDescriptor) Artifact(target TargetTriple) (PackageArtifact, error) {
	artifact, ok := d.Targets[target]
	if !ok {
		return PackageArtifact{}, fmt.Errorf("%w for %s", ErrMissingTarget, target)
	}
	return artifact, nil
}

// A ServiceInfo is what a resident service answers on its info path: the
// protocol it speaks and the operations it serves, each once.
//
//demi:wire
type ServiceInfo struct {
	ProtocolVersion uint64   `json:"protocolVersion" check:"eq=Version"`
	Operations      []string `json:"operations" check:"items=1..,unique,each(chars=1..)"`
}

// An ArtifactLocation says where a runner fetches an artifact: a URL, valid
// until ExpiresAt when set, or a path on the runner's machine. It holds one of
// the two.
//
// The URL is an HTTP or HTTPS URL without credentials: object storage answers
// with a signed HTTPS URL, and a development store with a URL on the backend
// itself. The scheme cannot change what runs, because the runner checks the
// download against the size and SHA-256 its pinned descriptor declares.
//
//demi:union untagged
//demi:msgpack
type ArtifactLocation interface {
	artifactLocation()
}

// An ArtifactURL locates an artifact by a URL.
//
//demi:variant
type ArtifactURL struct {
	URL string `json:"url" check:"chars=1..,func=downloadURL"`
	// ExpiresAt is the time the URL stops working, in milliseconds since the
	// Unix epoch.
	ExpiresAt *int64 `json:"expiresAt,omitzero"`
}

// An ArtifactPath locates an artifact by a path on the runner's machine.
//
//demi:variant
type ArtifactPath struct {
	Path string `json:"path" check:"chars=1.."`
}

func (ArtifactURL) artifactLocation()  {}
func (ArtifactPath) artifactLocation() {}
