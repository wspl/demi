package machinesproto

import (
	"errors"
	"fmt"
	"regexp"
	"runtime"
	"slices"
	"strings"

	"github.com/wspl/demi/go/commandservice"
)

// The paths inside every image (docs/cloud/images.md).
const (
	// RunnerPath is where the runner executable lives in every image.
	RunnerPath = "/usr/bin/demi-runner"
	// InitPath is the image's init, which runs the runner and reaps orphaned
	// processes.
	InitPath = "/usr/bin/tini"
	// ArtifactsPath is where an image embeds each command package's executable:
	// alone in a directory named by its SHA-256, under the name its release gives
	// it, such as /opt/demi/artifacts/<sha256>/demi-commands.
	ArtifactsPath = "/opt/demi/artifacts"
)

// runnerWireVersion is the runner wire's version, which a runner release names.
// It is the runner protocol's (crates/runner-protocol, wire::VERSION), which
// moves to the runnerproto package with the runner.
const runnerWireVersion = 24

// An Architecture is a Cloud image's CPU architecture. Persisted state never
// moves between architectures.
type Architecture string

// The architectures of Cloud images.
const (
	Amd64 Architecture = "amd64"
	Arm64 Architecture = "arm64"
)

// HostArchitecture returns the architecture this program was built for, and
// whether Cloud images exist for it.
func HostArchitecture() (Architecture, bool) {
	switch runtime.GOARCH {
	case "arm64":
		return Arm64, true
	case "amd64":
		return Amd64, true
	}
	return "", false
}

// Target returns the native target of the image's executables.
func (a Architecture) Target() commandservice.TargetTriple {
	if a == Arm64 {
		return "aarch64-unknown-linux-musl"
	}
	return "x86_64-unknown-linux-musl"
}

// A RootfsArchive is the base archive: its SHA-256, its byte size and its file.
//
//demi:wire
type RootfsArchive struct {
	SHA256 string `json:"sha256" check:"pattern=sha256Hex"`
	Size   uint64 `json:"size" check:"range=1.."`
	File   string `json:"file" check:"oneof=rootfs.tar.zst"`
}

// An InstalledPackage is a system package the image's dpkg database lists.
//
//demi:wire
type InstalledPackage struct {
	Name    string `json:"name" check:"chars=1.."`
	Version string `json:"version" check:"chars=1.."`
}

// A StandaloneTool is a tool the build installed, such as uv or Chrome.
//
//demi:wire
type StandaloneTool struct {
	Name    string `json:"name" check:"chars=1.."`
	Version string `json:"version" check:"chars=1.."`
	SHA256  string `json:"sha256" check:"pattern=sha256Hex"`
}

// A RunnerRelease is the runner release record an image embeds to name its
// runner (docs/delivery/builds-and-releases.md § Packaging). Its targets are the
// names of platforms, each with the artifact of its runner.
//
//demi:wire
type RunnerRelease struct {
	Release         string                                    `json:"release" check:"pattern=sha256Hex"`
	Wire            uint32                                    `json:"wire" check:"eq=runnerWireVersion"`
	CommandProtocol uint64                                    `json:"commandProtocol" check:"func=commandProtocol"`
	Targets         map[string]commandservice.PackageArtifact `json:"targets" check:"keys(func=commandservice.ValidateTarget),each(func=commandservice.Validate)"`
}

// A CloudImageManifest is a Cloud image release's manifest (images.md § Release
// artifacts): what a base archive holds and the build inputs it was made from.
// The SHA-256 of the exact manifest bytes is the base version.
//
//demi:wire
type CloudImageManifest struct {
	FormatVersion uint32        `json:"formatVersion" check:"eq=1"`
	OS            string        `json:"os" check:"oneof=linux"`
	Architecture  Architecture  `json:"architecture" check:"oneof=amd64|arm64"`
	Rootfs        RootfsArchive `json:"rootfs"`
	// Ubuntu is the Ubuntu release of the base.
	Ubuntu   string             `json:"ubuntu" check:"chars=1.."`
	Packages []InstalledPackage `json:"packages"`
	// Executables are the executables the image embeds, by absolute path under
	// /usr or /opt, with their size and SHA-256.
	Executables map[string]commandservice.PackageArtifact `json:"executables" check:"keys(func=imagePath),each(func=commandservice.Validate)"`
	// Releases are the command package releases whose artifacts the image
	// embeds.
	Releases []commandservice.PackageDescriptor `json:"releases" check:"each(func=commandservice.Validate)"`
	// Runner is the runner release of /usr/bin/demi-runner.
	Runner RunnerRelease    `json:"runner"`
	Tools  []StandaloneTool `json:"tools"`
}

// The errors of a manifest that another Cloud image release must not have.
var (
	// ErrRunner means the manifest's runner release does not identify the
	// embedded runner executable.
	ErrRunner = errors.New("Runner release must identify the embedded executable")
	// ErrArtifact means a command package's artifact for the image's target is
	// not embedded under its content-addressed path.
	ErrArtifact = errors.New("Missing embedded artifact")
)

// A ManifestError means a manifest is refused.
type ManifestError struct {
	// Release is the id of a command package whose artifact is not embedded,
	// when that is the refusal.
	Release string
	// Err says why.
	Err error
}

func (e *ManifestError) Error() string {
	if e.Release != "" {
		return "Missing embedded artifact for " + e.Release
	}
	return e.Err.Error()
}

func (e *ManifestError) Unwrap() error { return e.Err }

// The patterns and constants of the manifest's rules.
var sha256Hex = regexp.MustCompile(`^[0-9a-f]{64}$`)

// commandProtocol is the rule of the command protocol version a runner release
// names: the one this build speaks.
func commandProtocol(version uint64) error {
	if version != commandservice.Version {
		return fmt.Errorf("must be %d", commandservice.Version)
	}
	return nil
}

// DecodeManifest decodes a manifest's bytes and checks it: its values, that its
// runner release names the embedded runner and that each command package's
// artifact for the image's target is embedded under its content-addressed path.
// A refusal names the field and the rule.
func DecodeManifest(data []byte) (*CloudImageManifest, error) {
	manifest, err := decode[CloudImageManifest](data)
	if err != nil {
		return nil, &ManifestError{Err: fmt.Errorf("invalid Cloud image manifest: %w", err)}
	}
	if err := manifest.checkEmbedded(); err != nil {
		return nil, err
	}
	return &manifest, nil
}

// imagePath is the rule of an executable's path: an absolute path under /usr or
// /opt, on one line, with no ".." segment.
func imagePath(path string) error {
	under := strings.HasPrefix(path, "/usr/") || strings.HasPrefix(path, "/opt/")
	named := len(path) > len("/usr/") && !strings.ContainsAny(path, "\r\n")
	climbs := slices.Contains(strings.Split(path, "/"), "..")
	if !under || !named || climbs {
		return errors.New("is an invalid image executable path")
	}
	return nil
}

// checkEmbedded requires the runner release to name the embedded runner, and each
// command package's artifact for the image's target to be embedded under its
// content-addressed path.
func (m *CloudImageManifest) checkEmbedded() error {
	target := m.Architecture.Target()
	runner, haveRunner := m.Runner.Targets[string(target)]
	embedded, haveEmbedded := m.Executables[RunnerPath]
	if !haveRunner || !haveEmbedded || runner != embedded {
		return &ManifestError{Err: ErrRunner}
	}
	for _, release := range m.Releases {
		artifact, ok := release.Targets[target]
		if !ok {
			return &ManifestError{Release: release.ID, Err: ErrArtifact}
		}
		prefix := ArtifactsPath + "/" + artifact.SHA256 + "/"
		present := false
		for path, entry := range m.Executables {
			if strings.HasPrefix(path, prefix) && entry == artifact {
				present = true
				break
			}
		}
		if !present {
			return &ManifestError{Release: release.ID, Err: ErrArtifact}
		}
	}
	return nil
}
