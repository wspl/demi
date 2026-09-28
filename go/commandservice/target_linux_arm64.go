//go:build linux && arm64

package commandservice

// HostTarget identifies the native executable target of this build.
func HostTarget() string {
	return targetLinuxARM64
}
