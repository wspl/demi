//go:build linux && amd64

package commandservice

// HostTarget identifies the native executable target of this build.
func HostTarget() string {
	return targetLinuxAMD64
}
