package commandservice

import (
	"io/fs"
	"time"

	"golang.org/x/sys/unix"
)

// birthTime returns when the file at path was created, if the system and the
// file system tell: the standard library does not offer it on Linux, which
// reports it through statx.
func birthTime(path string, _ fs.FileInfo) (time.Time, bool) {
	var stat unix.Statx_t
	if err := unix.Statx(unix.AT_FDCWD, path, 0, unix.STATX_BTIME, &stat); err != nil || stat.Mask&unix.STATX_BTIME == 0 {
		return time.Time{}, false
	}
	return time.Unix(stat.Btime.Sec, int64(stat.Btime.Nsec)), true
}
