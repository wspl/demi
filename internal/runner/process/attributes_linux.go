package process

import (
	"os"
	"strconv"
	"strings"

	"golang.org/x/sys/unix"
)

// readUmask reads the Linux status without a process-wide mutation. As in
// Rust, a failed status read falls back to a brief stricter mask at startup.
func readUmask() uint32 {
	data, err := os.ReadFile("/proc/self/status")
	if err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			if value, ok := strings.CutPrefix(line, "Umask:"); ok {
				parsed, err := strconv.ParseUint(strings.TrimSpace(value), 8, 32)
				if err == nil {
					return uint32(parsed)
				}
			}
		}
	}
	previous := unix.Umask(0077)
	unix.Umask(previous)
	return uint32(previous)
}
