package process

import "golang.org/x/sys/unix"

// readUmask briefly makes the startup mask stricter while reading it. The
// runner calls Umask before starting jobs, and caches this value thereafter.
func readUmask() uint32 {
	previous := unix.Umask(0077)
	unix.Umask(previous)
	return uint32(previous)
}
