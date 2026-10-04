//go:build linux

package storage

import (
	"archive/tar"
	"context"
	"errors"
	"io"
	"os"
	"strings"

	"github.com/klauspost/compress/zstd"
)

// VetArchive checks every entry of a zstd-compressed tar archive before extraction.
func VetArchive(ctx context.Context, archive string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	file, err := os.Open(archive)
	if err != nil {
		return err
	}
	defer func() {
		_ = file.Close()
	}() // Read-only archive.
	// Synchronous streaming owns no decoder workers across namespace jobs.
	// Allow windows up to 2^27 bytes, libzstd's ZSTD_WINDOWLOG_LIMIT_DEFAULT.
	decoder, err := zstd.NewReader(file, zstd.WithDecoderConcurrency(1), zstd.WithDecoderMaxWindow(1<<27))
	if err != nil {
		return err
	}
	defer decoder.Close()
	reader := tar.NewReader(decoder)
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		entry, err := reader.Next()
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
		switch entry.Typeflag {
		case tar.TypeXGlobalHeader:
			continue
		case tar.TypeReg, tar.TypeDir, tar.TypeSymlink, tar.TypeLink:
		default:
			return ErrUnsafeEntry
		}
		if !archivePathInside(entry.Name) {
			return ErrUnsafeEntry
		}
		if entry.Typeflag == tar.TypeLink && (entry.Linkname == "" || !archivePathInside(entry.Linkname)) {
			return ErrUnsafeHardlink
		}
	}
}

// archivePathInside refuses absolute names and parent components in a Cloud archive.
func archivePathInside(name string) bool {
	if strings.HasPrefix(name, "/") {
		return false
	}
	for part := range strings.SplitSeq(name, "/") {
		if part == ".." {
			return false
		}
	}
	return true
}
