package host

import (
	"os"
	"time"

	"github.com/wspl/demi/internal/runnerwire"
)

// fileStat translates Host metadata. A pre-epoch modification time with a
// fractional millisecond is truncated toward zero (-1.5 ms becomes -1 ms).
func fileStat(info os.FileInfo) runnerwire.FileStat {
	modified := info.ModTime()
	millis := modified.UnixMilli()
	if modified.Unix() < 0 && modified.Nanosecond()%int(time.Millisecond) != 0 {
		millis++
	}
	result := runnerwire.FileStat{
		IsFile:         info.Mode().IsRegular(),
		IsDirectory:    info.IsDir(),
		IsSymbolicLink: info.Mode()&os.ModeSymlink != 0,
		Size:           uint64(info.Size()),
		Mtime:          runnerwire.Timestamp(millis),
	}
	nativeStat(info, &result)
	return result
}
