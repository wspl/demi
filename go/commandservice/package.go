package commandservice

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"net/url"
	"slices"

	"github.com/google/jsonschema-go/jsonschema"
)

// maxSafeInteger is the largest integer a JavaScript peer holds exactly.
const maxSafeInteger = 1<<53 - 1

// A PackageArtifact is one target's executable: its SHA-256 and size.
type PackageArtifact struct {
	SHA256 string `json:"sha256"`
	Size   uint64 `json:"size"`
}

var artifactWire = declare[PackageArtifact](func(s *jsonschema.Schema) {
	prop(s, "sha256").Pattern = `^[0-9a-f]{64}$`
	size := prop(s, "size")
	size.Minimum = jsonschema.Ptr(1.0)
	size.Maximum = jsonschema.Ptr(float64(maxSafeInteger))
}, nil)

// A PackageDescriptor is a native command package release: its identity, the
// operations it serves and the artifact of each target it carries.
// Publication requires every target; a development release may carry fewer.
type PackageDescriptor struct {
	ID              string                           `json:"id"`
	Version         string                           `json:"version"`
	ProtocolVersion uint64                           `json:"protocolVersion"`
	Operations      []string                         `json:"operations"`
	Targets         map[TargetTriple]PackageArtifact `json:"targets"`
}

var _ = declare[PackageDescriptor](func(s *jsonschema.Schema) {
	prop(s, "id").Pattern = `^[a-z0-9]+(?:[.-][a-z0-9]+)+$`
	prop(s, "version").MinLength = jsonschema.Ptr(1)
	prop(s, "protocolVersion").Const = jsonschema.Ptr[any](Version)
	limitOperations(prop(s, "operations"))
	prop(s, "targets").PropertyNames = &jsonschema.Schema{Enum: []any{
		string(TargetDarwinArm64), string(TargetDarwinAmd64),
		string(TargetLinuxArm64), string(TargetLinuxAmd64),
		string(TargetWindowsArm64), string(TargetWindowsAmd64),
	}}
}, nil, artifactWire)

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
type ServiceInfo struct {
	ProtocolVersion uint64   `json:"protocolVersion"`
	Operations      []string `json:"operations"`
}

var _ = declare[ServiceInfo](func(s *jsonschema.Schema) {
	prop(s, "protocolVersion").Const = jsonschema.Ptr[any](Version)
	limitOperations(prop(s, "operations"))
}, nil)

// An ArtifactLocation says where a runner fetches an artifact: a URL, valid
// until ExpiresAt when set, or a path on the runner's machine. It holds one of
// the two.
//
// The URL is an HTTP or HTTPS URL without credentials: object storage answers
// with a signed HTTPS URL, and a development store with a URL on the backend
// itself. The scheme cannot change what runs, because the runner checks the
// download against the size and SHA-256 its pinned descriptor declares.
type ArtifactLocation struct {
	URL string `json:"url,omitzero"`
	// ExpiresAt is the time the URL stops working, in milliseconds since the
	// Unix epoch.
	ExpiresAt *int64 `json:"expiresAt,omitzero"`
	Path      string `json:"path,omitzero"`
}

var _ = declare(func(s *jsonschema.Schema) {
	prop(s, "url").MinLength = jsonschema.Ptr(1)
	prop(s, "path").MinLength = jsonschema.Ptr(1)
	// A location with a path is a path and has nothing else; any other is a URL.
	s.If = &jsonschema.Schema{Required: []string{"path"}}
	s.Then = form{refuses: []string{"url", "expiresAt"}}.rules()
	s.Else = form{requires: []string{"url"}}.rules()
}, func(v *ArtifactLocation) error {
	if v.URL == "" {
		return nil
	}
	return checkDownloadURL(v.URL)
})

// checkDownloadURL refuses a URL a runner must not download from. It never
// echoes the URL, which may carry a signature.
func checkDownloadURL(raw string) error {
	address, err := url.Parse(raw)
	if err != nil {
		// url.Parse quotes the part of the URL it could not read.
		return errors.New("url: is not a valid URL")
	}
	web := address.Scheme == "http" || address.Scheme == "https"
	credentials := false
	if address.User != nil {
		_, hasPassword := address.User.Password()
		credentials = address.User.Username() != "" || hasPassword
	}
	if !web || address.Host == "" || credentials {
		return errors.New("url: artifact downloads require an HTTP or HTTPS URL without credentials")
	}
	return nil
}
