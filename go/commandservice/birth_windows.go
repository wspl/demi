package commandservice

import (
	"io/fs"
	"syscall"
	"time"
)

// birthTime returns when the file at path was created.
func birthTime(_ string, info fs.FileInfo) (time.Time, bool) {
	attributes, ok := info.Sys().(*syscall.Win32FileAttributeData)
	if !ok {
		return time.Time{}, false
	}
	return time.Unix(0, attributes.CreationTime.Nanoseconds()), true
}
