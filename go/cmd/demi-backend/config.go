package main

import (
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/user"
	"path/filepath"
	"strconv"

	whatwg "github.com/nlnwa/whatwg-url/url"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/edge"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/envflag"
	"github.com/wspl/demi/go/webapi"
)

// settings are the configuration as flags or DEMI_* variables
// (backend.md § Configuration), each value's name in --help and in every
// error its variable, so an unusable value stops startup naming it.
type settings struct {
	data           string
	port           uint16
	mode           webapi.InstanceMode
	publicURL      *whatwg.Url
	machinesSocket string
	nativeConfig   string
	objectStore    string
	// instanceSecret is checked by backendConfig, whose error leaves the
	// value out.
	instanceSecret *string
	exposeDomain   *backend.ExposeDomain
	webDirectory   string
	runnerReleases string
	claudeReleases *whatwg.Url
	log            logFilter
}

// The refusals of a configuration that parsed: values the program checks
// itself.
var (
	errNoDataDirectory = errors.New("DEMI_BACKEND_DATA is not set and the home directory is unknown")
	errInstanceSecret  = errors.New("DEMI_INSTANCE_SECRET must be 64 hexadecimal digits")
	errPublicURL       = errors.New("DEMI_BACKEND_PUBLIC_URL must be an HTTP or HTTPS URL without a user, a password, a query or a fragment")
)

// parseSettings reads args, the program's arguments after its name, and the
// variables lookup finds. -h and --help write the usage to help.
func parseSettings(args []string, lookup func(string) (string, bool), help io.Writer) (settings, error) {
	var s settings
	table := []*envflag.Setting{
		{Flag: "data", Variable: "DEMI_BACKEND_DATA", Parse: text(&s.data)},
		{Flag: "port", Variable: "DEMI_BACKEND_PORT", Fallback: "3271", Parse: port(&s.port)},
		{Flag: "mode", Variable: "DEMI_INSTANCE_MODE", Required: true, Parse: mode(&s.mode)},
		{Flag: "public-url", Variable: "DEMI_BACKEND_PUBLIC_URL", Required: true, Parse: absoluteURL(&s.publicURL)},
		{Flag: "machines-socket", Variable: "DEMI_MACHINES_SOCKET", Required: true, Parse: text(&s.machinesSocket)},
		{Flag: "native-config", Variable: "DEMI_NATIVE_CONFIG", Required: true, Parse: text(&s.nativeConfig)},
		{Flag: "object-store-config", Variable: "DEMI_OBJECT_STORE_CONFIG", Parse: text(&s.objectStore)},
		{Flag: "instance-secret", Variable: "DEMI_INSTANCE_SECRET", Parse: func(value string) error {
			s.instanceSecret = &value
			return nil
		}},
		{Flag: "expose-domain", Variable: "DEMI_EXPOSE_DOMAIN", Parse: exposeDomain(&s.exposeDomain)},
		{Flag: "web-directory", Variable: "DEMI_WEB_DIRECTORY", Parse: text(&s.webDirectory)},
		{Flag: "runner-release-dir", Variable: "DEMI_RUNNER_RELEASE_DIR", Parse: text(&s.runnerReleases)},
		{Flag: "claude-releases-url", Variable: "DEMI_CLAUDE_RELEASES_URL", Fallback: backend.DefaultClaudeReleasesURL, Parse: absoluteURL(&s.claudeReleases)},
		{Flag: "log", Variable: "DEMI_LOG", Fallback: "info", Parse: func(value string) (err error) {
			s.log, err = parseLogFilter(value)
			return err
		}},
	}
	command := envflag.Command{Name: "demi-backend", Settings: table, Usage: usage}
	if err := command.Parse(args, lookup, help); err != nil {
		return settings{}, err
	}
	return s, nil
}

func usage(out io.Writer) {
	fmt.Fprint(out, `The Demi product server

Usage: demi-backend [OPTIONS]

Options, each a flag or the variable it names:
  --data <DEMI_BACKEND_DATA>                        The data directory [default: ~/.demi/backend]
  --port <DEMI_BACKEND_PORT>                        The TCP port the backend listens on, 1 to 65535 [default: 3271]
  --mode <DEMI_INSTANCE_MODE>                       shared or isolated: who configures providers
  --public-url <DEMI_BACKEND_PUBLIC_URL>            The URL runners and Cloud guests connect to
  --machines-socket <DEMI_MACHINES_SOCKET>          The machine manager's Unix socket
  --native-config <DEMI_NATIVE_CONFIG>              The native command releases and the object storage they are published to
  --object-store-config <DEMI_OBJECT_STORE_CONFIG>  A JSON file that puts the object store in an S3 bucket
  --instance-secret <DEMI_INSTANCE_SECRET>          The instance secret as 64 hexadecimal digits [default: generated into the data directory]
  --expose-domain <DEMI_EXPOSE_DOMAIN>              The domain of expose hostnames; without it, exposes are unavailable
  --web-directory <DEMI_WEB_DIRECTORY>              A built browser directory to serve beside the API
  --runner-release-dir <DEMI_RUNNER_RELEASE_DIR>    The runner releases the installer routes serve
  --claude-releases-url <DEMI_CLAUDE_RELEASES_URL>  The Claude Code distribution whose newest release the CLI on each Cloud follows [default: `+backend.DefaultClaudeReleasesURL+`]
  --log <DEMI_LOG>                                  What the backend logs: a level, and a level per target, comma-separated [default: info]
  -h, --help                                        Print help
`)
}

// backendConfig is what edge.Start takes for these settings, on the system
// clock, listening on every address at the port.
func (s settings) backendConfig() (edge.Config, error) {
	data := s.data
	if data == "" {
		home, err := homeDirectory()
		if err != nil {
			return edge.Config{}, errNoDataDirectory
		}
		data = filepath.Join(home, ".demi", "backend")
	}
	var secret *backend.InstanceSecret
	if s.instanceSecret != nil {
		parsed, err := backend.ParseInstanceSecret(*s.instanceSecret)
		if err != nil {
			return edge.Config{}, errInstanceSecret
		}
		secret = &parsed
	}
	address := net.JoinHostPort("0.0.0.0", strconv.Itoa(int(s.port)))
	config := edge.NewConfig(data, address, s.mode, s.machinesSocket)
	config.InstanceSecret = secret
	config.WebDirectory = s.webDirectory
	config.ExposeDomain = s.exposeDomain
	if !backend.InstallableURL(s.publicURL) {
		return edge.Config{}, errPublicURL
	}
	config.PublicURL = s.publicURL
	config.RunnerReleases = s.runnerReleases
	config.ObjectStore = s.objectStore
	releases, err := core.NetURL(s.claudeReleases)
	if err != nil {
		return edge.Config{}, fmt.Errorf("DEMI_CLAUDE_RELEASES_URL %w", err)
	}
	config.ClaudeReleases = *releases
	return config, nil
}

// homeDirectory is the user's home: $HOME, else the account database's.
func homeDirectory() (string, error) {
	if home, err := os.UserHomeDir(); err == nil {
		return home, nil
	}
	account, err := user.Current()
	if err != nil || account.HomeDir == "" {
		return "", errors.New("the home directory is unknown")
	}
	return account.HomeDir, nil
}

func text(target *string) func(string) error {
	return func(value string) error {
		*target = value
		return nil
	}
}

func port(target *uint16) func(string) error {
	return func(value string) error {
		number, err := strconv.ParseInt(value, 10, 64)
		if err != nil {
			return errors.New("must be a number")
		}
		if number < 1 || number > 65535 {
			return fmt.Errorf("%d is not in 1..=65535", number)
		}
		*target = uint16(number)
		return nil
	}
}

func mode(target *webapi.InstanceMode) func(string) error {
	return func(value string) error {
		switch mode := webapi.InstanceMode(value); mode {
		case webapi.InstanceModeShared, webapi.InstanceModeIsolated:
			*target = mode
			return nil
		}
		return errors.New("must be shared or isolated")
	}
}

func absoluteURL(target **whatwg.Url) func(string) error {
	return func(value string) error {
		address, err := core.ParseURL(value)
		if err != nil {
			return err
		}
		*target = address
		return nil
	}
}

func exposeDomain(target **backend.ExposeDomain) func(string) error {
	return func(value string) error {
		domain, err := backend.ParseExposeDomain(value)
		if err != nil {
			return err
		}
		*target = &domain
		return nil
	}
}
