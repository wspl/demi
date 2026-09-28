//go:build windows && arm64

package commandservice

// HostTarget identifies the native executable target of this build.
func HostTarget() string {
	return targetWindowsARM64
}
