//go:build !linux && !darwin && !windows

package commandservice

import (
	"io/fs"
	"time"
)

// birthTime returns nothing: this system's creation time is not read.
func birthTime(string, fs.FileInfo) (time.Time, bool) {
	return time.Time{}, false
}
