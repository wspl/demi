package host

import "errors"

func syncFilesystems(_ []ManagedVolume) error {
	return errors.New("whole-filesystem sync is unavailable for a directory runner on Windows")
}

func volumeUsage(_ string) (uint64, uint64, error) {
	return 0, 0, errors.New("managed block volumes require Linux")
}
