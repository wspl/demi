package storage

import (
	"archive/tar"
	"errors"
	"io"
	"os"
	"strings"

	"github.com/klauspost/compress/zstd"
)

// The refusals of a base archive.
var (
	// ErrUnsafeEntry means an entry is not a regular file, directory, symbolic
	// link or hard link, or its path is absolute or climbs out with "..".
	ErrUnsafeEntry = errors.New("Cloud archive contains an unsafe path or entry type")
	// ErrUnsafeHardlink means a hard link's target is absolute or climbs out with
	// "..".
	ErrUnsafeHardlink = errors.New("Cloud archive contains an unsafe hardlink")
)

// VetArchive reads every entry of the zstd-compressed tar archive at path
// (docs/cloud/images.md § Import and publication): each is a regular file,
// directory, symbolic link or hard link, and no path or hard-link target is
// absolute or contains "..".
func VetArchive(path string) error {
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	decoder, err := zstd.NewReader(file)
	if err != nil {
		return err
	}
	defer decoder.Close()
	archive := tar.NewReader(decoder)
	for {
		header, err := archive.Next()
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
		switch header.Typeflag {
		case tar.TypeXGlobalHeader:
			// Metadata for the whole archive, not a member.
			continue
		case tar.TypeReg, tar.TypeDir, tar.TypeSymlink, tar.TypeLink:
		default:
			return ErrUnsafeEntry
		}
		if !staysInside(header.Name) {
			return ErrUnsafeEntry
		}
		if header.Typeflag == tar.TypeLink && !staysInside(header.Linkname) {
			return ErrUnsafeHardlink
		}
	}
}

// staysInside reports whether path names something beneath the extraction root:
// it is not absolute and has no ".." segment.
func staysInside(path string) bool {
	if strings.HasPrefix(path, "/") {
		return false
	}
	for _, segment := range strings.Split(path, "/") {
		if segment == ".." {
			return false
		}
	}
	return true
}
