package commandservice

import (
	"io/fs"
	"syscall"
	"time"
)

// birthTime returns when the file at path was created.
func birthTime(_ string, info fs.FileInfo) (time.Time, bool) {
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		return time.Time{}, false
	}
	return time.Unix(stat.Birthtimespec.Unix()), true
}
