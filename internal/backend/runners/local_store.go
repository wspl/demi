package runners

import (
	"context"
	"errors"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/gates"
)

// NativeArtifactsRoute is the development downloads' route at the public URL's root.
const NativeArtifactsRoute = "/native-artifacts"

// ArtifactFile pairs a release's file with the artifact its descriptor declares.
type ArtifactFile struct {
	// Path is the artifact's local file path.
	Path string
	// Artifact is the declared size and SHA-256 of the file.
	Artifact commandproto.PackageArtifact
}

// LocalArtifacts holds loaded development artifacts by SHA-256. Construct it with
// NewLocalArtifacts; concurrent requests share each executable's first encoding.
type (
	LocalArtifacts struct{ files map[string]*localFile }
	localFile      struct {
		path    string
		size    uint64
		encode  bool
		gate    gates.Serial
		encoded []byte
	}
)

// NewLocalArtifacts records executable and resource archive files, keeping an
// artifact shared by several releases once. Archives are served without encoding.
func NewLocalArtifacts(executables, archives []ArtifactFile) *LocalArtifacts {
	files := make(map[string]*localFile)
	for _, executable := range executables {
		if _, exists := files[executable.Artifact.SHA256]; !exists {
			files[executable.Artifact.SHA256] = &localFile{
				path:   executable.Path,
				size:   executable.Artifact.Size,
				encode: true,
			}
		}
	}
	for _, archive := range archives {
		if _, exists := files[archive.Artifact.SHA256]; !exists {
			files[archive.Artifact.SHA256] = &localFile{path: archive.Path, size: archive.Artifact.Size}
		}
	}
	return &LocalArtifacts{files: files}
}

// Artifact returns a loaded artifact, and false when no release carries sha256.
// Executables are encoded lazily; files are verified when releases are loaded.
func (a *LocalArtifacts) Artifact(ctx context.Context, sha256 string) (LocalArtifact, bool, error) {
	file := a.files[sha256]
	if file == nil {
		return nil, false, nil
	}
	if !file.encode {
		return &PlainArtifact{Path: file.path}, true, nil
	}
	permit, err := file.gate.Acquire(ctx)
	if err != nil {
		return nil, false, err
	}
	defer permit.Release()
	if file.encoded == nil {
		bytes, err := os.ReadFile(file.path)
		if err != nil {
			return nil, false, fmt.Errorf("%s: %w", file.path, err)
		}
		encoded, err := artifacts.Encode(ctx, bytes, artifacts.Development)
		if err != nil {
			return nil, false, err
		}
		file.encoded = encoded
	}
	return &EncodedArtifact{Bytes: file.encoded}, true, nil
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

func (*EncodedArtifact) localArtifact() {}
func (*PlainArtifact) localArtifact()   {}

// ServedArtifacts resolves loaded development artifacts on the backend's public URL.
type ServedArtifacts struct {
	// Artifacts is the store of loaded artifacts.
	Artifacts *LocalArtifacts
	// Backend is the address of the backend serving them.
	Backend *PublicURL
}

// Resolve locates artifact on the development backend without an expiry.
func (a *ServedArtifacts) Resolve(
	_ context.Context,
	artifact commandproto.PackageArtifact,
	_ string,
) (commandproto.ArtifactLocation, error) {
	file := a.Artifacts.files[artifact.SHA256]
	if file == nil || file.size != artifact.Size {
		return nil, errors.New("the artifact is not in a loaded development release")
	}
	backend, ok := a.Backend.URL()
	if !ok {
		return nil, errors.New("the backend does not listen yet")
	}
	parsed, err := backend.URL()
	if err != nil {
		return nil, err
	}
	return &commandproto.ArtifactURL{
		URL: parsed.Scheme() + "://" + parsed.Host() + NativeArtifactsRoute + "/" + artifact.SHA256,
	}, nil
}
