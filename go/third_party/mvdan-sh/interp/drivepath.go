package interp

import "runtime"

// DrivePath converts a path whose first component names a Windows drive, /c
// or /c/rest, to the drive's own form, c:/rest, as MSYS shells and Demi's Rust
// runner do. On other systems, and for any other path, it returns path.
func DrivePath(path string) string {
	if runtime.GOOS != "windows" {
		return path
	}
	return drivePath(path)
}

// drivePath is DrivePath's conversion on every system.
func drivePath(path string) string {
	if len(path) < 2 || path[0] != '/' || !(path[1] >= 'a' && path[1] <= 'z' || path[1] >= 'A' && path[1] <= 'Z') || len(path) > 2 && path[2] != '/' {
		return path
	}
	rest := ""
	if len(path) > 3 {
		rest = path[3:]
	}
	return path[1:2] + ":/" + rest
}
