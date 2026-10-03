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

// NativeConfig is `DEMI_NATIVE_CONFIG`: the releases, and the store runners download their
// executables from.
// +demi:root
// +demi:check checkNativeConfig
type NativeConfig struct {
	// The key prefix of every object published to S3; the local store
	// takes none.
	// +demi:nullable
	Prefix   *string         `json:"prefix,omitempty"`
	Releases []NativeRelease `json:"releases"`
	Store    NativeStore     `json:"store"`
}

// NativeRelease is one release directory: `descriptor.json`, and one executable per target
// triple, named `executable` (with `.exe` for Windows).
type NativeRelease struct {
	Directory  string `json:"directory"`
	Executable string `json:"executable"`
}

// NativeStore is where runners download the executables: object storage the releases
// are published to, where S3 is the one protocol, or a development store,
// which is the backend itself.
// +demi:union tag=provider
//
//sumtype:decl
type NativeStore interface{ nativeConfigStore() }

// S3NativeStore publishes the releases to an S3 bucket.
// +demi:variant NativeStore s3
type S3NativeStore struct{ blobs.S3Config }

func (*S3NativeStore) nativeConfigStore() {}

// LocalNativeStore is a development store's settings: it has none.
// +demi:variant NativeStore local
type LocalNativeStore struct{}

func (*LocalNativeStore) nativeConfigStore() {}

var nativePrefix = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]*(/[A-Za-z0-9_-]+)*$`)
var executableBasename = regexp.MustCompile(`^[A-Za-z0-9_-]+$`)

// prefix supplies the default namespace for published native objects.
func (c NativeConfig) prefix() string {
	if c.Prefix != nil {
		return *c.Prefix
	}
	return "native"
}

// checkNativeConfig checks the store-specific namespace and executable basenames.
func checkNativeConfig(c NativeConfig) error {
	switch store := c.Store.(type) {
	case *S3NativeStore:
		if !nativePrefix.MatchString(c.prefix()) {
			return fmt.Errorf("%s is no object key prefix", c.prefix())
		}
		if err := store.S3Config.Validate(); err != nil {
			return err
		}
	case *LocalNativeStore:
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
func readNativeConfig(ctx context.Context, path string) (NativeConfig, error) {
	if err := ctx.Err(); err != nil {
		return NativeConfig{}, &PublicationError{Kind: PublicationCancelled, Err: err}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return NativeConfig{}, &PublicationError{Kind: PublicationConfig, Reason: path + ": " + err.Error(), Err: err}
	}
	config, err := DecodeNativeConfig(data)
	if err != nil {
		return NativeConfig{}, &PublicationError{Kind: PublicationConfig, Reason: path + ": " + err.Error(), Err: err}
	}
	for i := range config.Releases {
		if !filepath.IsAbs(config.Releases[i].Directory) {
			config.Releases[i].Directory = filepath.Join(filepath.Dir(path), config.Releases[i].Directory)
		}
	}
	return config, nil
}
