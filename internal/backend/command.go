package backend

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"io"
	"net/url"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/webapi"
)

// CLIConfig is the configuration as flags or DEMI_* variables. Each value's
// name in --help and every error is its variable, so an unusable value stops
// startup naming the variable. Config is the composition boundary for tests.
type CLIConfig struct {
	// The data directory [default: ~/.demi/backend]
	Data *string
	// The TCP port the backend listens on, 1 to 65535
	Port uint16
	// `shared` or `isolated`: who configures providers
	Mode webapi.InstanceMode
	// The URL runners and Cloud guests connect to
	PublicURL *url.URL
	// The machine manager's Unix socket
	MachinesSocket string
	// The native command releases and the object storage they are published to
	NativeConfig string
	// A JSON file that puts the object store in an S3 bucket
	ObjectStoreConfig *string
	// The instance secret as 64 hexadecimal digits [default: generated into the data directory]
	InstanceSecret *string
	// The domain of expose hostnames; without it, exposes are unavailable
	ExposeDomain *expose.Domain
	// The web app build's directory, to serve beside the API
	WebDirectory *string
	// The runner releases the installer routes serve
	RunnerReleaseDir *string
	// The Claude Code distribution whose newest release the CLI on each Cloud follows
	ClaudeReleasesURL *url.URL
	// What the backend logs: a level, and a level per target, comma-separated,
	// such as `info,demi::provider::claude_code::wire=trace`
	Log string
}

// ParseConfig parses command arguments (without argv[0]) and NAME=value
// environment entries. Unknown DEMI_* names are refused using cli's rule.
// All input is validated before returning. Help and version are handled by Main.
func ParseConfig(args, environ []string) (CLIConfig, error) { panic("not written: b-backend") }

// Backend resolves the data directory and instance secret and returns the
// composition configuration on the system clock. Native publication belongs
// to Main and runs before Start.
func (c CLIConfig) Backend() (Config, error) { panic("not written: b-backend") }

// WriteHelp lists the backend's flags and environment variables without
// revealing environment values.
func WriteHelp(w io.Writer) error { panic("not written: b-backend") }

// Main runs demi-backend using the process's arguments, environment and
// standard streams, reporting release for --version. It watches interrupt
// and termination signals before startup, publishes native releases, starts
// the backend and joins ordered shutdown before returning its exit status.
func Main(ctx context.Context, release string) int { panic("not written: b-backend") }

// ConfigErrorKind identifies a configuration value that prevents startup.
type ConfigErrorKind uint8

const (
	// ConfigNoDataDirectory means neither a data path nor a home is known.
	ConfigNoDataDirectory ConfigErrorKind = iota
	// ConfigInstanceSecret means the supplied secret is malformed.
	ConfigInstanceSecret
	// ConfigPublicURL means the backend URL is not an installation URL.
	ConfigPublicURL
	// ConfigArgument means a flag or variable failed typed parsing.
	ConfigArgument
	// ConfigUnknownVariable means a DEMI_* name is not a backend setting.
	ConfigUnknownVariable
)

// ConfigError names an unusable configuration setting without exposing secrets.
type ConfigError struct {
	Kind     ConfigErrorKind
	Variable string
	Err      error
}

// Error describes the configuration failure.
func (e *ConfigError) Error() string { panic("not written: b-backend") }

// Unwrap returns the underlying parsing failure, if any.
func (e *ConfigError) Unwrap() error { panic("not written: b-backend") }
