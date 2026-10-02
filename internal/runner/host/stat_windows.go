package host

import (
	"os"

	"github.com/wspl/demi/internal/runnerwire"
)

func nativeStat(info os.FileInfo, result *runnerwire.FileStat) {
	mode := uint32(0100000)
	if info.IsDir() {
		mode = 0040000 | 0111
	} else if info.Mode()&os.ModeSymlink != 0 {
		mode = 0120000
	}
	if info.Mode().Perm()&0200 == 0 {
		mode |= 0444
	} else {
		mode |= 0666
	}
	result.Mode = mode
}
