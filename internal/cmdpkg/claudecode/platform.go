package claudecode

import (
	"os"
	"runtime"
)

type loaders struct{ musl, glibc bool }

func platformKey(system, arch string, loaded loaders) string {
	var cpu string
	switch arch {
	case "amd64":
		cpu = "x64"
	case "arm64":
		cpu = "arm64"
	default:
		return ""
	}
	switch system {
	case "darwin":
		return "darwin-" + cpu
	case "windows":
		return "win32-" + cpu
	case "linux":
		key := "linux-" + cpu
		if loaded.musl && !loaded.glibc {
			key += "-musl"
		}
		return key
	default:
		return ""
	}
}

func currentPlatform() string {
	var loaded loaders
	if runtime.GOOS == "linux" {
		var musl, glibc string
		switch runtime.GOARCH {
		case "amd64":
			musl, glibc = "/lib/ld-musl-x86_64.so.1", "/lib64/ld-linux-x86-64.so.2"
		case "arm64":
			musl, glibc = "/lib/ld-musl-aarch64.so.1", "/lib/ld-linux-aarch64.so.1"
		default:
			return ""
		}
		_, err := os.Stat(musl)
		loaded.musl = err == nil
		_, err = os.Stat(glibc)
		loaded.glibc = err == nil
	}
	return platformKey(runtime.GOOS, runtime.GOARCH, loaded)
}
