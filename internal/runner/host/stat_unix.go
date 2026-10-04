//go:build darwin || linux

package host

import (
	"os"
	"syscall"

	"github.com/wspl/demi/internal/runnerproto"
)

func nativeStat(info os.FileInfo, result *runnerproto.FileStat) {
	// os.Stat and os.Lstat on Unix document Sys as *syscall.Stat_t.
	native := info.Sys().(*syscall.Stat_t)
	uid, gid := native.Uid, native.Gid
	ino, dev, nlink := uint64(native.Ino), uint64(native.Dev), uint64(native.Nlink)
	character := info.Mode()&os.ModeCharDevice != 0
	fifo := info.Mode()&os.ModeNamedPipe != 0
	result.Mode = uint32(native.Mode)
	result.UID = &uid
	result.GID = &gid
	result.Ino = &ino
	result.Dev = &dev
	result.Nlink = &nlink
	result.IsCharacterDevice = &character
	result.IsFIFO = &fifo
}
