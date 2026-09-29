package claude

import (
	"os"
	"runtime"
)

// Loaders says which dynamic loaders a Linux machine has for its architecture.
type Loaders struct {
	Musl  bool
	Glibc bool
}

// PlatformKey returns the distribution platform key for an operating system and
// an architecture, named as runtime.GOOS and runtime.GOARCH name them, or false
// when the CLI has no build for them. A Linux machine takes the -musl build only
// when it has a musl loader and no glibc loader: this package is a static build
// that also runs on glibc machines, and the two CLI builds are not
// interchangeable.
func PlatformKey(goos, goarch string, loaders Loaders) (string, bool) {
	musl := loaders.Musl && !loaders.Glibc
	switch [2]string{goos, goarch} {
	case [2]string{"darwin", "arm64"}:
		return "darwin-arm64", true
	case [2]string{"darwin", "amd64"}:
		return "darwin-x64", true
	case [2]string{"windows", "amd64"}:
		return "win32-x64", true
	case [2]string{"windows", "arm64"}:
		return "win32-arm64", true
	case [2]string{"linux", "amd64"}:
		if musl {
			return "linux-x64-musl", true
		}
		return "linux-x64", true
	case [2]string{"linux", "arm64"}:
		if musl {
			return "linux-arm64-musl", true
		}
		return "linux-arm64", true
	}
	return "", false
}

// CurrentPlatform returns this machine's platform key, or false when the CLI has
// no build for it.
func CurrentPlatform() (string, bool) {
	var loaders Loaders
	if runtime.GOOS == "linux" {
		var musl, glibc string
		switch runtime.GOARCH {
		case "amd64":
			musl, glibc = "/lib/ld-musl-x86_64.so.1", "/lib64/ld-linux-x86-64.so.2"
		case "arm64":
			musl, glibc = "/lib/ld-musl-aarch64.so.1", "/lib/ld-linux-aarch64.so.1"
		default:
			return "", false
		}
		loaders = Loaders{Musl: exists(musl), Glibc: exists(glibc)}
	}
	return PlatformKey(runtime.GOOS, runtime.GOARCH, loaders)
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
