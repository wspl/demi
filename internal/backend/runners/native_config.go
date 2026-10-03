package runners

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"regexp"

	"github.com/wspl/demi/internal/backend/blobs"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// `DEMI_NATIVE_CONFIG`: the releases, and the store runners download their
// executables from.
// +demi:root
// +demi:check checkNativeConfig
type nativeConfig struct {
	// The key prefix of every object published to S3; the local store
	// takes none.
	// +demi:nullable
	Prefix   *string         `json:"prefix,omitempty"`
	Releases []nativeRelease `json:"releases"`
	Store    nativeStore     `json:"store"`
}

// One release directory: `descriptor.json`, and one executable per target
// triple, named `executable` (with `.exe` for Windows).
type nativeRelease struct {
	Directory  string `json:"directory"`
	Executable string `json:"executable"`
}

// Where runners download the executables: object storage the releases
// are published to, where S3 is the one protocol, or a development store,
// which is the backend itself.
// +demi:union tag=provider
//
//sumtype:decl
type nativeStore interface{ nativeConfigStore() }

// +demi:variant nativeStore s3
type s3NativeStore struct{ blobs.S3Config }

func (*s3NativeStore) nativeConfigStore() {}

// A development store's settings: it has none.
// +demi:variant nativeStore local
type localNativeStore struct{}

func (*localNativeStore) nativeConfigStore() {}

var nativePrefix = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]*(/[A-Za-z0-9_-]+)*$`)
var executableBasename = regexp.MustCompile(`^[A-Za-z0-9_-]+$`)

// prefix supplies the default namespace for published native objects.
func (c nativeConfig) prefix() string {
	if c.Prefix != nil {
		return *c.Prefix
	}
	return "native"
}

// checkNativeConfig checks the store-specific namespace and executable basenames.
func checkNativeConfig(c nativeConfig) error {
	switch store := c.Store.(type) {
	case *s3NativeStore:
		if !nativePrefix.MatchString(c.prefix()) {
			return fmt.Errorf("%s is no object key prefix", c.prefix())
		}
		if err := store.S3Config.Validate(); err != nil {
			return err
		}
	case *localNativeStore:
		if c.Prefix != nil {
			return fmt.Errorf("prefix names object storage keys, and the local store has none")
		}
	}
	for _, release := range c.Releases {
		if !executableBasename.MatchString(release.Executable) {
			return fmt.Errorf("%s is no executable basename", release.Executable)
		}
	}
	return nil
}

// readNativeConfig resolves release directories against the configuration file.
func readNativeConfig(ctx context.Context, path string) (nativeConfig, error) {
	if err := ctx.Err(); err != nil {
		return nativeConfig{}, &PublicationError{Kind: PublicationCancelled, Err: err}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nativeConfig{}, &PublicationError{Kind: PublicationConfig, Reason: path + ": " + err.Error(), Err: err}
	}
	config, err := decodeNativeConfig(data)
	if err != nil {
		return nativeConfig{}, &PublicationError{Kind: PublicationConfig, Reason: path + ": " + err.Error(), Err: err}
	}
	for i := range config.Releases {
		if !filepath.IsAbs(config.Releases[i].Directory) {
			config.Releases[i].Directory = filepath.Join(filepath.Dir(path), config.Releases[i].Directory)
		}
	}
	return config, nil
}
