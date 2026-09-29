package machinesproto

import (
	"fmt"
	"regexp"
)

// An IDError means a name breaks the image-name rule.
type IDError struct {
	// Kind says what the name was to name: "device id", "generation" or
	// "base version".
	Kind  string
	Value string
}

func (e *IDError) Error() string {
	return fmt.Sprintf("invalid %s: %q", e.Kind, e.Value)
}

// imageName is the rule of a name of a device, a generation or a base: one or
// more ASCII letters, digits, '_' or '-'. Such a name is always one path
// component, so it names a directory under the manager's state directory.
var imageName = regexp.MustCompile(`^[A-Za-z0-9_-]+$`)

// IsImageName reports whether value may name a device, a generation or a base.
func IsImageName(value string) bool {
	return imageName.MatchString(value)
}

// A DeviceID is a persistent Cloud device, as the backend names it. It names
// the device's image and working directories.
type DeviceID string

// A GenerationID names one committed pair of a device's system and home images.
type GenerationID string

// A BaseVersion names an imported base: the SHA-256 of its image manifest's
// bytes.
type BaseVersion string

// ParseDeviceID checks value against the image-name rule.
func ParseDeviceID(value string) (DeviceID, error) {
	if !IsImageName(value) {
		return "", &IDError{Kind: "device id", Value: value}
	}
	return DeviceID(value), nil
}

// ParseGenerationID checks value against the image-name rule.
func ParseGenerationID(value string) (GenerationID, error) {
	if !IsImageName(value) {
		return "", &IDError{Kind: "generation", Value: value}
	}
	return GenerationID(value), nil
}

// ParseBaseVersion checks value against the image-name rule.
func ParseBaseVersion(value string) (BaseVersion, error) {
	if !IsImageName(value) {
		return "", &IDError{Kind: "base version", Value: value}
	}
	return BaseVersion(value), nil
}
