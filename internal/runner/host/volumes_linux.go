package host

import (
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

func syncFilesystems(blocks []ManagedVolume) error {
	if len(blocks) == 0 {
		unix.Sync()
		return nil
	}
	for _, block := range blocks {
		file, err := os.Open(block.Mount)
		if err != nil {
			return err
		}
		err = unix.Syncfs(int(file.Fd()))
		if err = errors.Join(err, file.Close()); err != nil {
			return err
		}
	}
	return nil
}

func volumeUsage(path string) (uint64, uint64, error) {
	var stat unix.Statfs_t
	if err := unix.Statfs(path, &stat); err != nil {
		return 0, 0, err
	}
	// Linux statvfs derives fragment size from statfs, falling back to Bsize.
	fragment := uint64(stat.Frsize)
	if fragment == 0 {
		fragment = uint64(stat.Bsize)
	}
	if fragment != 0 && stat.Blocks > ^uint64(0)/fragment {
		return 0, 0, errors.New("volume capacity overflow")
	}
	if fragment != 0 && stat.Bavail > ^uint64(0)/fragment {
		return 0, 0, errors.New("volume free-space overflow")
	}
	return stat.Blocks * fragment, stat.Bavail * fragment, nil
}
