package commandsdk

import (
	"os"
	"syscall"
	"time"
)

func creationStamp(_ string, info os.FileInfo) time.Time {
	stat, ok := info.Sys().(*syscall.Win32FileAttributeData)
	if !ok {
		return time.Time{}
	}
	return time.Unix(0, stat.CreationTime.Nanoseconds())
}
