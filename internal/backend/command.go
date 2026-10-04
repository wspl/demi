package backend

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net/netip"
	"net/url"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cli"
	"github.com/wspl/demi/internal/webapiproto"
)

// CLIConfig is the configuration as flags or DEMI_* variables. Each value's
// name in --help and every error is its variable, so an unusable value stops
// startup naming the variable. Config is the composition boundary for tests.
type CLIConfig struct {
	// Data is the data directory [default: ~/.demi/backend].
	Data *string
	// Port is the TCP port the backend listens on, 1 to 65535.
	Port uint16
	// Mode selects `shared` or `isolated`: who configures providers.
	Mode webapiproto.InstanceMode
	// PublicURL is the URL runners and Cloud guests connect to.
	PublicURL *url.URL
	// MachinesSocket is the machine manager's Unix socket.
	MachinesSocket string
	// NativeConfig is the native command releases and the object storage they are published to.
	NativeConfig string
	// ObjectStoreConfig is a JSON file that puts the object store in an S3 bucket.
	ObjectStoreConfig *string
	// InstanceSecret is the instance secret as 64 hexadecimal digits [default: generated into the data directory].
	InstanceSecret *string
	// ExposeDomain is the domain of expose hostnames; without it, exposes are unavailable.
	ExposeDomain *expose.Domain
	// WebDirectory is the web app build's directory, to serve beside the API.
	WebDirectory *string
	// RunnerReleaseDir is the runner releases the installer routes serve.
	RunnerReleaseDir *string
	// ClaudeReleasesURL is the Claude Code distribution whose newest release the CLI on each Cloud follows.
	ClaudeReleasesURL *url.URL
	// Log selects what the backend logs: a level, and a level per target, comma-separated,
	// such as `info,provider.claudecode.wire=trace`.
	Log string
}

// ParseConfig parses command arguments (without argv[0]) and NAME=value
// environment entries. Unknown DEMI_* names are refused using cli's rule.
// All input is validated before returning. Help and version are handled by Main.
func ParseConfig(args, environ []string) (CLIConfig, error) {
	values, present, names, err := parseCLIFlags(args, environ)
	if err != nil {
		return CLIConfig{}, err
	}
	optional := func(name string) *string {
		if present[name] {
			return values[name]
		}
		return nil
	}
	c := CLIConfig{
		Data:              optional("data"),
		Mode:              webapiproto.InstanceMode(*values["mode"]),
		MachinesSocket:    *values["machines-socket"],
		NativeConfig:      *values["native-config"],
		ObjectStoreConfig: optional("object-store-config"),
		InstanceSecret:    optional("instance-secret"),
		WebDirectory:      optional("web-directory"),
		RunnerReleaseDir:  optional("runner-release-dir"),
		Log:               *values["log"],
	}
	c, err = validateCLIConfig(c, values, optional)
	if err != nil {
		return CLIConfig{}, err
	}
	if name, found := cli.UnknownVariable("DEMI_", names, environ); found {
		return CLIConfig{}, errors.New(name + " is not a backend setting; `demi-backend --help` lists them")
	}
	return c, nil
}

// Backend resolves the data directory and instance secret and returns the
// composition configuration on the system clock. Native publication belongs
// to Main and runs before Start.
func (c CLIConfig) Backend() (Config, error) {
	var data string
	if c.Data != nil {
		data = *c.Data
	} else {
		home, err := os.UserHomeDir()
		if err != nil {
			return Config{}, errors.New("DEMI_BACKEND_DATA is not set and the home directory is unknown")
		}
		data = filepath.Join(home, ".demi", "backend")
	}
	var secret *InstanceSecret
	if c.InstanceSecret != nil {
		value, err := ParseInstanceSecret(*c.InstanceSecret)
		if err != nil {
			return Config{}, errInstanceSecretSetting
		}
		secret = &value
	}
	if c.PublicURL == nil {
		return Config{}, errPublicURLSetting
	}
	public, err := runners.BackendURL(c.PublicURL)
	if err != nil {
		return Config{}, errPublicURLSetting
	}
	config, err := NewConfig(data, netip.AddrPortFrom(netip.IPv4Unspecified(), c.Port), c.Mode, c.MachinesSocket)
	if err != nil {
		return Config{}, err
	}
	config.InstanceSecret = secret
	config.PublicURL = public
	config.ExposeDomain = c.ExposeDomain
	config.ClaudeReleases = c.ClaudeReleasesURL
	if c.ObjectStoreConfig != nil {
		config.ObjectStore = *c.ObjectStoreConfig
		config.objectStoreSet = true
	}
	if c.WebDirectory != nil {
		config.WebDirectory = *c.WebDirectory
		if config.WebDirectory == "" {
			config.WebDirectory = "."
		}
	}
	if c.RunnerReleaseDir != nil {
		config.RunnerReleases = *c.RunnerReleaseDir
		if config.RunnerReleases == "" {
			config.RunnerReleases = "."
		}
	}
	return config, nil
}

// WriteHelp lists the backend's flags and environment variables without
// revealing environment values.
func WriteHelp(w io.Writer) error {
	if _, err := fmt.Fprintln(
		w,
		"The Demi product server\n\n"+
			"Usage: demi-backend [OPTIONS] --mode <DEMI_INSTANCE_MODE> "+
			"--public-url <DEMI_BACKEND_PUBLIC_URL> --machines-socket <DEMI_MACHINE_MANAGER_SOCKET> "+
			"--native-config <DEMI_NATIVE_CONFIG>\n\nOptions:",
	); err != nil {
		return err
	}
	for _, setting := range settings {
		if _, err := fmt.Fprintf(
			w,
			"      --%s <%s>\n          %s [env: %s]\n",
			setting.flag,
			setting.variable,
			setting.help,
			setting.variable,
		); err != nil {
			return err
		}
	}
	_, err := fmt.Fprintln(w, "  -h, --help     Print help\n  -V, --version  Print version")
	return err
}

// Main runs demi-backend using the process's arguments, environment and
// standard streams, reporting release for --version. It watches interrupt
// and termination signals before startup, publishes native releases, starts
// the backend and joins ordered shutdown before returning its exit status.
func Main(ctx context.Context, release string) (status int) {
	stopCtx, stop := signal.NotifyContext(ctx, os.Interrupt, syscall.SIGTERM)
	defer stop()
	fail := func(err error) int {
		// A failed diagnostic write cannot change the required failure exit status.
		_, _ = fmt.Fprintln(os.Stderr, "demi-backend: "+err.Error())
		return 1
	}
	if displayed, err := displayCLIInfo(release); displayed {
		if err != nil {
			return fail(err)
		}
		return 0
	}
	config, err := ParseConfig(os.Args[1:], os.Environ())
	if err != nil {
		return fail(err)
	}
	filter, err := parseLogTargets(config.Log)
	if err != nil {
		return fail(err)
	}
	previous := installLogTargets(filter)
	defer slog.SetDefault(previous)
	settings, err := config.Backend()
	if err != nil {
		return fail(err)
	}
	native, err := runners.PublishNative(stopCtx, config.NativeConfig)
	if err != nil {
		return fail(err)
	}
	settings.Native = native
	// Publication belongs to the executable; the backend borrows its catalog.
	defer func() {
		if err := native.Close(context.WithoutCancel(ctx)); err != nil {
			status = fail(err)
		}
	}()
	// A stop during startup is remembered; it does not interrupt recovery.
	b, err := Start(context.WithoutCancel(ctx), settings)
	if err != nil {
		return fail(err)
	}
	slog.Info(
		"demi-backend is listening",
		"address",
		b.LocalAddr().String(),
		"data",
		settings.DataDir,
		"mode",
		config.Mode,
	)
	<-stopCtx.Done()
	if err := b.Close(context.WithoutCancel(ctx)); err != nil {
		return fail(err)
	}
	return 0
}

var (
	errInstanceSecretSetting = errors.New("DEMI_INSTANCE_SECRET must be 64 hexadecimal digits")
	errPublicURLSetting      = errors.New(
		"DEMI_BACKEND_PUBLIC_URL must be an HTTP or HTTPS URL without a user, a password, a query or a fragment",
	)
)

// settings is the one declaration of flags, environment names and help text.
var settings = []struct {
	flag, variable, initial, help string
	required                      bool
}{
	{"data", "DEMI_BACKEND_DATA", "", "The data directory [default: ~/.demi/backend]", false},
	{"port", "DEMI_BACKEND_PORT", "3271", "The TCP port the backend listens on, 1 to 65535", false},
	{"mode", "DEMI_INSTANCE_MODE", "", "`shared` or `isolated`: who configures providers", true},
	{"public-url", "DEMI_BACKEND_PUBLIC_URL", "", "The URL runners and Cloud guests connect to", true},
	{"machines-socket", "DEMI_MACHINE_MANAGER_SOCKET", "", "The machine manager's Unix socket", true},
	{
		"native-config",
		"DEMI_NATIVE_CONFIG",
		"",
		"The native command releases and the object storage they are published to",
		true,
	},
	{
		"object-store-config",
		"DEMI_OBJECT_STORE_CONFIG",
		"",
		"A JSON file that puts the object store in an S3 bucket",
		false,
	},
	{
		"instance-secret",
		"DEMI_INSTANCE_SECRET",
		"",
		"The instance secret as 64 hexadecimal digits [default: generated into the data directory]",
		false,
	},
	{
		"expose-domain",
		"DEMI_EXPOSE_DOMAIN",
		"",
		"The domain of expose hostnames; without it, exposes are unavailable",
		false,
	},
	{"web-directory", "DEMI_WEB_DIRECTORY", "", "The web app build's directory, to serve beside the API", false},
	{"runner-release-dir", "DEMI_RUNNER_RELEASE_DIR", "", "The runner releases the installer routes serve", false},
	{
		"claude-releases-url",
		"DEMI_CLAUDE_RELEASES_URL",
		providerhost.DefaultReleasesURL,
		"The Claude Code distribution whose newest release the CLI on each Cloud follows",
		false,
	},
	{
		"log",
		"DEMI_LOG",
		"info",
		"What the backend logs: a level, and a level per target, comma-separated, " +
			"such as `info,provider.claudecode.wire=trace`",
		false,
	},
}

// validateCLIConfig checks typed flag values in their diagnostic order.
func validateCLIConfig(c CLIConfig, values map[string]*string, optional func(string) *string) (CLIConfig, error) {
	port, err := strconv.ParseUint(*values["port"], 10, 16)
	if err != nil || port == 0 {
		return CLIConfig{}, errors.New("DEMI_BACKEND_PORT: must be an integer from 1 to 65535")
	}
	c.Port = uint16(port)
	if err := c.Mode.Validate(); err != nil {
		return CLIConfig{}, fmt.Errorf("DEMI_INSTANCE_MODE: %w", err)
	}
	c.PublicURL, err = url.Parse(*values["public-url"])
	if err != nil || c.PublicURL.Scheme == "" {
		return CLIConfig{}, errPublicURLSetting
	}
	if _, err = runners.BackendURL(c.PublicURL); err != nil {
		return CLIConfig{}, errPublicURLSetting
	}
	c.ClaudeReleasesURL, err = url.Parse(*values["claude-releases-url"])
	if err != nil || c.ClaudeReleasesURL.Scheme == "" {
		return CLIConfig{}, errors.New("DEMI_CLAUDE_RELEASES_URL: relative URL without a base")
	}
	if value := optional("expose-domain"); value != nil {
		domain, err := expose.ParseDomain(*value)
		if err != nil {
			return CLIConfig{}, fmt.Errorf("DEMI_EXPOSE_DOMAIN: %w", err)
		}
		c.ExposeDomain = &domain
	}
	if c.InstanceSecret != nil {
		if _, err := ParseInstanceSecret(*c.InstanceSecret); err != nil {
			return CLIConfig{}, errInstanceSecretSetting
		}
	}
	if _, err := parseLogTargets(c.Log); err != nil {
		return CLIConfig{}, fmt.Errorf("DEMI_LOG: %w", err)
	}
	return c, nil
}

// displayCLIInfo handles help and version before parsing configuration.
func displayCLIInfo(release string) (bool, error) {
	for _, arg := range os.Args[1:] {
		if arg == "--help" || arg == "-h" {
			if err := WriteHelp(os.Stdout); err != nil {
				return true, err
			}
			return true, nil
		}
		if arg == "--version" || arg == "-V" {
			if _, err := fmt.Fprintln(os.Stdout, "demi-backend "+release); err != nil {
				return true, err
			}
			return true, nil
		}
	}
	return false, nil
}

// parseCLIFlags applies environment defaults and checks required backend flags.
func parseCLIFlags(args, environ []string) (map[string]*string, map[string]bool, []string, error) {
	flags := flag.NewFlagSet("demi-backend", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	values := make(map[string]*string)
	present := make(map[string]bool)
	env := make(map[string]string)
	for _, entry := range environ {
		name, value, ok := strings.Cut(entry, "=")
		if ok {
			env[name] = value
		}
	}
	names := make([]string, 0, len(settings))
	for _, setting := range settings {
		value := setting.initial
		if fromEnv, found := env[setting.variable]; found {
			value = fromEnv
			present[setting.flag] = true
		}
		values[setting.flag] = flags.String(setting.flag, value, setting.help)
		names = append(names, setting.variable)
	}
	if err := flags.Parse(args); err != nil {
		return nil, nil, nil, err
	}
	if flags.NArg() != 0 {
		return nil, nil, nil, fmt.Errorf("unexpected argument '%s' found", flags.Arg(0))
	}
	flags.Visit(func(f *flag.Flag) {
		present[f.Name] = true
	})
	for _, setting := range settings {
		if setting.required && !present[setting.flag] {
			return nil, nil, nil, fmt.Errorf("%s: a value is required", setting.variable)
		}
	}
	return values, present, names, nil
}

// installLogTargets configures process logging and returns the previous logger for restoration.
func installLogTargets(filter []logTarget) *slog.Logger {
	previous := slog.Default()
	slog.SetDefault(
		slog.New(
			&targetHandler{
				next:    slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.Level(-8)}),
				targets: filter,
			},
		),
	)
	return previous
}
