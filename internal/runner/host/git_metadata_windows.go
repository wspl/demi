package host

import (
	"os"

	"github.com/go-git/go-git/v5/plumbing/format/index"
)

func metadataMatches(_ os.FileInfo, _ *index.Entry, _ int64) bool {
	// Windows FileInfo does not expose the index's change-time/device/inode tuple.
	// Hashing avoids hiding same-size content changes with a restored mtime.
	return false
}
