package machines

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"net/netip"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/cli"
	"github.com/wspl/demi/internal/runnerwire"
)

// RuntimeDirectory is where the manager keeps runtime bundles, locks and its namespace handle.
const RuntimeDirectory = "/run/demi-machine-manager"

// Mode is what the manager was started to do.
type Mode uint8

const (
	// ModeServe recovers, then serves until shutdown.
	ModeServe Mode = iota
	// ModeRecover fences and saves a stopped manager's devices.
	ModeRecover
	// ModeRecoverNamespace runs inside the saved namespace as the manager's child.
	ModeRecoverNamespace
)

// Limits holds one sandbox's cgroup v2 budgets.
type Limits struct {
	CPUs      uint32
	MemoryMiB uint32
}

// MemoryBytes returns the memory budget in bytes.
func (l Limits) MemoryBytes() uint64 { return uint64(l.MemoryMiB) << 20 }

// Config is the validated manager configuration.
type Config struct {
	Mode                       Mode
	Socket, Data, Runsc, Image string
	BackendURL                 runnerwire.BackendURL
	Limits                     *Limits
	SystemMiB, HomeMiB         uint32
	Subnet                     netip.Prefix
	Slots                      uint16
	DNS                        []netip.Addr
}

// Working returns the devices' working pairs directory.
func (c Config) Working() string { return filepath.Join(c.Data, "working") }

// Images returns the bases and committed generations directory.
func (c Config) Images() string { return filepath.Join(c.Data, "images") }

// Runtime returns the manager's fixed runtime directory.
func (c Config) Runtime() string { return RuntimeDirectory }

// SystemBytes returns a new system filesystem's capacity.
func (c Config) SystemBytes() uint64 { return uint64(c.SystemMiB) << 20 }

// HomeBytes returns a new home filesystem's capacity.
func (c Config) HomeBytes() uint64 { return uint64(c.HomeMiB) << 20 }

// ConfigFromEnv reads process arguments and environment.
func ConfigFromEnv() (Config, error) { return ParseConfig(os.Args[1:], os.Environ()) }

// ParseConfig reads flags (without argv[0]) and environment, rejecting unknown managed settings.
func ParseConfig(args, environ []string) (c Config, err error) {
	defer func() {
		var display *ConfigDisplay
		var setting *ConfigError
		if err != nil && !errors.As(err, &display) && !errors.As(err, &setting) {
			err = &InvalidConfigError{Cause: err}
		}
	}()
	settings := []struct{ flag, name, initial, help string }{
		{"socket", "DEMI_MACHINE_MANAGER_SOCKET", "", "The socket the backend connects to; required to serve."},
		{"data", "DEMI_MACHINE_MANAGER_DATA", "/var/lib/demi-machine-manager", "The persistent state directory, on one filesystem."},
		{"runsc", "DEMI_MANAGED_RUNSC", "", "The pinned runsc executable."},
		{"image", "DEMI_MANAGED_IMAGE", "", "The directory holding the Cloud image manifest and archive."},
		{"backend-url", "DEMI_MANAGED_BACKEND_URL", "", "The only backend a sandbox's runner may connect to."},
		{"limits", "DEMI_MANAGED_LIMITS", "on", "Whether sandboxes run under cgroup v2 CPU, memory and PID limits."},
		{"cpus", "DEMI_MANAGED_CPUS", "2", "The CPU budget of one sandbox, with the limits on."},
		{"mem-mib", "DEMI_MANAGED_MEM_MIB", "2048", "The memory limit of one sandbox, in MiB, with the limits on."},
		{"system-mib", "DEMI_MANAGED_SYSTEM_MIB", "1024", "A new system filesystem's capacity, in MiB."},
		{"home-mib", "DEMI_MANAGED_HOME_MIB", "1024", "A new home filesystem's capacity, in MiB."},
		{"subnet", "DEMI_MANAGED_SUBNET", "172.30.0.0/16", "The IPv4 pool the sandboxes' networks come from."},
		{"slots", "DEMI_MANAGED_SLOTS", "256", "How many sandboxes may run at once; each takes four addresses."},
		{"dns", "DEMI_MANAGED_DNS", "", "The resolvers a sandbox uses, separated by commas."},
	}
	names := make([]string, 0, len(settings))
	env := map[string]string{}
	for _, entry := range environ {
		k, v, _ := strings.Cut(entry, "=")
		env[k] = v
	}
	for _, s := range settings {
		names = append(names, s.name)
	}
	if name, ok := cli.UnknownVariable("DEMI_MANAGED_", names, environ); ok {
		return c, &ConfigError{Message: fmt.Sprintf("%s is not a Cloud manager setting", name)}
	}
	for _, arg := range args {
		if arg == "--version" || arg == "-V" {
			return c, &ConfigDisplay{Text: "demi-machine-manager 0.1.3\n"}
		}
		if arg == "--help" || arg == "-h" {
			var help strings.Builder
			help.WriteString("The Cloud machine manager: runs users' Cloud machines as gVisor\nsandboxes and serves the backend over a Unix socket.\n\nUsage: demi-machine-manager [OPTIONS]\n\nOptions:\n")
			help.WriteString("  --recover\n      Fence and save what a stopped manager left behind, then exit (the\n      service's stop-post command).\n")
			for _, setting := range settings {
				fmt.Fprintf(&help, "  --%s <%s>\n      %s\n", setting.flag, setting.name, setting.help)
				if setting.initial != "" {
					fmt.Fprintf(&help, "      [default: %s]\n", setting.initial)
				}
			}
			help.WriteString("  -h, --help\n      Print help\n  -V, --version\n      Print version\n")
			return c, &ConfigDisplay{Text: help.String()}
		}
	}
	fs := flag.NewFlagSet("demi-machine-manager", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	values := map[string]*string{}
	explicit := map[string]bool{}
	for _, s := range settings {
		value := s.initial
		if v, ok := env[s.name]; ok {
			value = v
			explicit[s.flag] = true
		}
		values[s.flag] = fs.String(s.flag, value, s.help)
	}
	recovery := fs.Bool("recover", false, "Fence and save what a stopped manager left behind, then exit (the service's stop-post command).")
	namespace := fs.Bool("recover-namespace", false, "Recover inside a saved mount namespace; only the manager starts this.")
	if err := fs.Parse(args); err != nil {
		return c, err
	}
	if fs.NArg() != 0 {
		return c, fmt.Errorf("unexpected argument: %s", fs.Arg(0))
	}
	fs.Visit(func(f *flag.Flag) { explicit[f.Name] = true })
	if *recovery && *namespace {
		return c, errors.New("--recover conflicts with --recover-namespace")
	}
	if *recovery {
		c.Mode = ModeRecover
	}
	if *namespace {
		c.Mode = ModeRecoverNamespace
	}
	for _, s := range settings {
		if *values[s.flag] == "" && s.flag != "socket" {
			return c, fmt.Errorf("%s is required", s.name)
		}
	}
	c.Socket = *values["socket"]
	c.Data = *values["data"]
	c.Runsc = *values["runsc"]
	c.Image = *values["image"]
	for _, s := range settings[:4] {
		v := *values[s.flag]
		if v != "" && !filepath.IsAbs(v) {
			return c, fmt.Errorf("%s: must be an absolute path", s.name)
		}
	}
	if c.Mode == ModeServe && c.Socket == "" {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, &ConfigError{Message: "DEMI_MACHINE_MANAGER_SOCKET is required"}
	}
	backend, err := runnerwire.ParseBackendURL(*values["backend-url"])
	if err != nil {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, fmt.Errorf("DEMI_MANAGED_BACKEND_URL: %w", err)
	}
	if !strings.HasPrefix(backend.String(), "http://") && !strings.HasPrefix(backend.String(), "https://") {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, errors.New("DEMI_MANAGED_BACKEND_URL: must be an http or https URL")
	}
	c.BackendURL = backend
	numbers := map[string]uint32{}
	for _, s := range settings[6:10] {
		v := *values[s.flag]
		if strings.Trim(v, "0123456789") != "" || v == "" {
			return c, fmt.Errorf("%s: must be a positive decimal integer", s.name)
		}
		n, e := strconv.ParseUint(v, 10, 32)
		if e != nil || n == 0 {
			return c, fmt.Errorf("%s: must be a positive decimal integer", s.name)
		}
		numbers[s.flag] = uint32(n)
	}
	c.SystemMiB = numbers["system-mib"]
	c.HomeMiB = numbers["home-mib"]
	switch *values["limits"] {
	case "on":
		c.Limits = &Limits{CPUs: numbers["cpus"], MemoryMiB: numbers["mem-mib"]}
	case "off":
		for _, s := range settings[6:8] {
			if explicit[s.flag] {
				return c, &ConfigError{Message: fmt.Sprintf("%s applies only with DEMI_MANAGED_LIMITS=on", s.name)}
			}
		}
	default:
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, errors.New("DEMI_MANAGED_LIMITS: must be on or off")
	}
	c.Subnet, err = netip.ParsePrefix(*values["subnet"])
	if err != nil || !c.Subnet.Addr().Is4() || c.Subnet.Bits() < 8 || c.Subnet.Bits() > 30 || c.Subnet != c.Subnet.Masked() {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, errors.New("DEMI_MANAGED_SUBNET: must be an aligned IPv4 network with prefix /8 through /30")
	}
	v := *values["slots"]
	n, err := strconv.ParseUint(v, 10, 16)
	if err != nil || strings.Trim(v, "0123456789") != "" || n < 1 || n > 16384 {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, errors.New("DEMI_MANAGED_SLOTS: must be from 1 to 16384")
	}
	c.Slots = uint16(n)
	if n*4 > uint64(1)<<(32-c.Subnet.Bits()) {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return c, &ConfigError{Message: "DEMI_MANAGED_SLOTS exceeds DEMI_MANAGED_SUBNET capacity"}
	}
	for _, v := range strings.Split(*values["dns"], ",") {
		a, e := netip.ParseAddr(v)
		if e != nil || !a.Is4() || a.As4()[0] == 0 || a.IsLoopback() || a.IsMulticast() || a == netip.AddrFrom4([4]byte{255, 255, 255, 255}) {
			//nolint:staticcheck // User-visible text is copied verbatim from Rust.
			return c, errors.New("DEMI_MANAGED_DNS: resolver must be reachable IPv4")
		}
		c.DNS = append(c.DNS, a)
	}
	return c, nil
}

// ConfigDisplay is a successful help or version request, handled before startup.
type ConfigDisplay struct{ Text string }

func (d *ConfigDisplay) Error() string { return d.Text }

// ConfigError is a manager-specific configuration rejection, reported with exit status 1.
type ConfigError struct{ Message string }

func (e *ConfigError) Error() string { return e.Message }

// InvalidConfigError is a malformed command-line value, reported with exit status 2.
type InvalidConfigError struct{ Cause error }

func (e *InvalidConfigError) Error() string { return e.Cause.Error() }
func (e *InvalidConfigError) Unwrap() error { return e.Cause }
