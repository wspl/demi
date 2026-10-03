package cmdsdk

import (
	"os"
	"time"

	"golang.org/x/sys/unix"
)

// statx supplies the optional creation time that ordinary Linux stat omits.
func creationStamp(path string, _ os.FileInfo) time.Time {
	var stat unix.Statx_t
	if err := unix.Statx(
		unix.AT_FDCWD,
		path,
		0,
		unix.STATX_BTIME,
		&stat,
	); err != nil ||
		stat.Mask&unix.STATX_BTIME == 0 {
		return time.Time{}
	}
	return time.Unix(stat.Btime.Sec, int64(stat.Btime.Nsec))
}
