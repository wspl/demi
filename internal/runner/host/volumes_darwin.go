package host

import (
	"errors"

	"golang.org/x/sys/unix"
)

func syncFilesystems(_ []ManagedVolume) error {
	return unix.Sync()
}

func volumeUsage(_ string) (uint64, uint64, error) {
	return 0, 0, errors.New("managed block volumes require Linux")
}
