package host

import (
	"os"
	"syscall"
	"time"

	"github.com/go-git/go-git/v5/plumbing/format/index"
)

func metadataMatches(info os.FileInfo, e *index.Entry, indexTime int64) bool {
	native := info.Sys().(*syscall.Stat_t) // os.FileInfo.Sys on Darwin is documented as Stat_t.
	return info.Size() == int64(e.Size) && info.ModTime().Equal(e.ModifiedAt) && info.ModTime().UnixNano() < indexTime && time.Unix(native.Ctimespec.Unix()).Equal(e.CreatedAt) && uint32(native.Ino) == e.Inode && uint32(native.Dev) == e.Dev
}
