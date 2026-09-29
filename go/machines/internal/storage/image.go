package storage

import (
	"path/filepath"

	"github.com/wspl/demi/go/machinesproto"
)

// An ImagePair holds a value for each of a device's two volumes.
type ImagePair[T any] struct {
	System T
	Home   T
}

// Get returns the value of volume.
func (p ImagePair[T]) Get(volume machinesproto.Volume) T {
	if volume == machinesproto.System {
		return p.System
	}
	return p.Home
}

// ImageFile returns the file name of volume's image in a generation, a working
// pair or a stage: system.ext4 or home.ext4.
func ImageFile(volume machinesproto.Volume) string {
	return string(volume) + ".ext4"
}

// ImagesIn returns the images named system.ext4 and home.ext4 in directory.
func ImagesIn(directory string) ImagePair[string] {
	return ImagePair[string]{
		System: filepath.Join(directory, ImageFile(machinesproto.System)),
		Home:   filepath.Join(directory, ImageFile(machinesproto.Home)),
	}
}
