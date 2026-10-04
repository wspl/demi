package host

import (
	"os"

	"github.com/wspl/demi/internal/runnerproto"
)

func nativeStat(info os.FileInfo, result *runnerproto.FileStat) {
	mode := uint32(0o100000)
	if info.IsDir() {
		mode = 0o040000 | 0o111
	} else if info.Mode()&os.ModeSymlink != 0 {
		mode = 0o120000
	}
	if info.Mode().Perm()&0o200 == 0 {
		mode |= 0o444
	} else {
		mode |= 0o666
	}
	result.Mode = mode
}
