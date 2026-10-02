package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
)

// NativeArtifactsRoute is the development downloads' route at the public URL's root.
const NativeArtifactsRoute = "/native-artifacts"

// ArtifactFile pairs a release's file with the artifact its descriptor declares.
type ArtifactFile struct {
	// Path is the artifact's local file path.
	Path string
	// Artifact is the declared size and SHA-256 of the file.
	Artifact commandwire.PackageArtifact
}

// LocalArtifacts holds loaded development artifacts by SHA-256. Construct it with
// NewLocalArtifacts; concurrent requests share each executable's first encoding.
type LocalArtifacts struct{}

// NewLocalArtifacts records executable and resource archive files, keeping an
// artifact shared by several releases once. Archives are served without encoding.
func NewLocalArtifacts(executables, archives []ArtifactFile) *LocalArtifacts {
	panic("not written: b-runners")
}

// Artifact returns a loaded artifact, or nil when no release carries sha256.
// Executables are encoded lazily; files are verified when releases are loaded.
func (a *LocalArtifacts) Artifact(ctx context.Context, sha256 string) (LocalArtifact, error) {
	panic("not written: b-runners")
}

// LocalArtifact is how the development store serves an artifact.
//
//sumtype:decl
type LocalArtifact interface{ localArtifact() }

// EncodedArtifact is an executable in artifacts.ContentCoding.
type EncodedArtifact struct {
	// Bytes are the encoded executable, shared immutably by concurrent downloads.
	Bytes []byte
}

// PlainArtifact is a resource archive served as its original file.
type PlainArtifact struct {
	// Path names the archive file; the HTTP response owns any reader it opens.
	Path string
}

func (*EncodedArtifact) localArtifact() { panic("not written: b-runners") }
func (*PlainArtifact) localArtifact()   { panic("not written: b-runners") }

// ServedArtifacts resolves loaded development artifacts on the backend's public URL.
type ServedArtifacts struct {
	// Artifacts is the store of loaded artifacts.
	Artifacts *LocalArtifacts
	// Backend is the address of the backend serving them.
	Backend *PublicURL
}

// Resolve locates artifact on the development backend without an expiry.
func (a *ServedArtifacts) Resolve(ctx context.Context, artifact commandwire.PackageArtifact, target string) (commandwire.ArtifactLocation, error) {
	panic("not written: b-runners")
}
