package machinewire

import (
	"errors"
	"fmt"

	"runtime"
	"strings"

	"github.com/wspl/demi/internal/runnerwire"
)

// Where the runner executable lives in every image.
const RunnerPath = "/usr/bin/demi-runner"

// The image's init, which runs the runner and reaps orphaned processes.
const InitPath = "/usr/bin/tini"

const (
	ArchitectureAMD64 Architecture = "amd64"
	ArchitectureARM64 Architecture = "arm64"
	OSLinux           OS           = "linux"
	RootfsTarZst      RootfsFile   = "rootfs.tar.zst"
)

// ErrManifestRunner means the runner release does not identify the embedded executable.
var ErrManifestRunner = errors.New("Runner release must identify the embedded executable")

// A manifest the manager or the packaging command refuses.
type ManifestError struct{ Release string }

func (e *ManifestError) Error() string {
	return fmt.Sprintf("Missing embedded artifact for %s", e.Release)
}

// The architecture this program was built for, if Cloud images exist
// for it.
func HostArchitecture() (Architecture, bool) {
	switch runtime.GOARCH {
	case "amd64":
		return ArchitectureAMD64, true
	case "arm64":
		return ArchitectureARM64, true
	default:
		return "", false
	}
}

// The native target of the image's executables.
func (a Architecture) Target() string {
	switch a {
	case ArchitectureAMD64:
		return "x86_64-unknown-linux-musl"
	case ArchitectureARM64:
		return "aarch64-unknown-linux-musl"
	default:
		return ""
	}
}

// validateManifest ties the image's releases to the executables in its archive.
func validateManifest(m CloudImageManifest) error {
	for path := range m.Executables {
		under := strings.HasPrefix(path, "/usr/") || strings.HasPrefix(path, "/opt/")
		if !under || len(path) <= len("/usr/") || strings.ContainsAny(path, "\r\n") {
			return fmt.Errorf("invalid image executable path %q", path)
		}
		for _, segment := range strings.Split(path, "/") {
			if segment == ".." {
				return fmt.Errorf("invalid image executable path %q", path)
			}
		}
	}
	target := m.Architecture.Target()
	runner, ok := m.Runner.Targets[target]
	embedded, present := m.Executables[RunnerPath]
	if !ok || !present || runner != embedded {
		return ErrManifestRunner
	}
	for _, release := range m.Releases {
		artifact, ok := release.Targets[target]
		if !ok {
			return &ManifestError{Release: release.ID}
		}
		prefix := runnerwire.ArtifactsPath + "/" + artifact.SHA256 + "/"
		found := false
		for path, entry := range m.Executables {
			if strings.HasPrefix(path, prefix) && entry == artifact {
				found = true
				break
			}
		}
		if !found {
			return &ManifestError{Release: release.ID}
		}
	}
	return nil
}

// Name returns the base archive's file name.
func (f RootfsFile) Name() string { return string(f) }
