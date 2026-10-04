package machinemanagerproto

import (
	"errors"
	"fmt"
	"runtime"
	"strings"

	"github.com/wspl/demi/internal/runnerproto"
)

// RunnerPath is where the runner executable lives in every image.
const RunnerPath = "/usr/bin/demi-runner"

// InitPath names the image's init, which runs the runner and reaps orphaned processes.
const InitPath = "/usr/bin/tini"

// Supported image architectures, operating system and archive file.
const (
	ArchitectureAMD64 Architecture = "amd64"
	ArchitectureARM64 Architecture = "arm64"
	OSLinux           OS           = "linux"
	RootfsTarZst      RootfsFile   = "rootfs.tar.zst"
)

// ErrManifestRunner means the runner release does not identify the embedded executable.
//
//nolint:staticcheck // User-visible text, kept byte for byte.
var ErrManifestRunner = errors.New("Runner release must identify the embedded executable")

// HostArchitecture returns the build architecture if Cloud images exist for it.
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

// Target returns the native target of the image's executables.
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
			//nolint:staticcheck // User-visible text, kept byte for byte.
			return fmt.Errorf("Missing embedded artifact for %s", release.ID)
		}
		prefix := runnerproto.ArtifactsPath + "/" + artifact.SHA256 + "/"
		found := false
		for path, entry := range m.Executables {
			if strings.HasPrefix(path, prefix) && entry == artifact {
				found = true
				break
			}
		}
		if !found {
			//nolint:staticcheck // User-visible text, kept byte for byte.
			return fmt.Errorf("Missing embedded artifact for %s", release.ID)
		}
	}
	return nil
}

// Name returns the base archive's file name.
func (f RootfsFile) Name() string {
	return string(f)
}
