// Package config reads the machine manager's configuration
// (docs/cloud/setup.md § Configuration): its command line and the
// DEMI_MACHINES_* and DEMI_MANAGED_* variables, validated at startup. An invalid
// or unknown setting stops the manager with an error that names the variable.
package config

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"net/netip"
	"net/url"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/runnerproto"
)

// RuntimeDirectory is where the manager keeps its runtime bundles, locks and
// namespace handle.
const RuntimeDirectory = "/run/demi-machines"

// managedPrefix is the prefix of the variables the manager owns: one it does not
// know is an error, so a setting from another runtime cannot be silently
// ignored.
const managedPrefix = "DEMI_MANAGED_"

// A Mode says what the manager was started to do.
type Mode int

// The modes.
const (
	// Serve recovers, then serves the backend until SIGTERM (the service).
	Serve Mode = iota
	// Recover fences and saves a stopped manager's devices, then exits.
	Recover
	// RecoverNamespace does the same inside a saved mount namespace, as the
	// manager's own child.
	RecoverNamespace
)

// Limits are one sandbox's cgroup v2 limits (docs/cloud/managed-hosts.md §
// Resource limits).
type Limits struct {
	CPUs      uint32
	MemoryMiB uint32
}

// MemoryBytes returns the memory limit in bytes.
func (l Limits) MemoryBytes() uint64 { return uint64(l.MemoryMiB) << 20 }

// A Config is the validated configuration.
type Config struct {
	Mode Mode
	// Socket is present in [Serve].
	Socket     string
	Data       string
	Runsc      string
	Image      string
	BackendURL *url.URL
	// Limits are each sandbox's cgroup limits; nil with DEMI_MANAGED_LIMITS=off,
	// which runs sandboxes without cgroups.
	Limits    *Limits
	SystemMiB uint32
	HomeMiB   uint32
	Subnet    netip.Prefix
	Slots     uint16
	DNS       []netip.Addr
}

// Working returns the devices' working pairs: <data>/working.
func (c *Config) Working() string { return filepath.Join(c.Data, "working") }

// Images returns bases and committed generations: <data>/images.
func (c *Config) Images() string { return filepath.Join(c.Data, "images") }

// Runtime returns the runtime directory.
func (c *Config) Runtime() string { return RuntimeDirectory }

// SystemBytes returns a new system filesystem's capacity in bytes.
func (c *Config) SystemBytes() uint64 { return uint64(c.SystemMiB) << 20 }

// HomeBytes returns a new home filesystem's capacity in bytes.
func (c *Config) HomeBytes() uint64 { return uint64(c.HomeMiB) << 20 }

// Errors of a configuration that no single value explains.
var (
	// ErrSlotsExceedSubnet means the slots do not fit the pool.
	ErrSlotsExceedSubnet = errors.New("DEMI_MANAGED_SLOTS exceeds DEMI_MANAGED_SUBNET capacity")
	// ErrMissingSocket means serving needs a socket.
	ErrMissingSocket = errors.New("DEMI_MACHINES_SOCKET is required")
)

// A UsageError means the command line or a setting's value is not one the
// manager reads, as clap says of its own refusals; the other errors of a
// configuration are the manager's (the Rust exits with status 2 for the first and
// 1 for the second).
type UsageError struct {
	Err error
}

func (e *UsageError) Error() string { return e.Err.Error() }

func (e *UsageError) Unwrap() error { return e.Err }

// IsUsage reports whether err is a [UsageError].
func IsUsage(err error) bool {
	var usage *UsageError
	return errors.As(err, &usage)
}

// An UnknownError means a DEMI_MANAGED_* variable the manager does not declare.
type UnknownError struct {
	Name string
}

func (e *UnknownError) Error() string { return e.Name + " is not a Cloud manager setting" }

// A LimitsOffError means a CPU budget or memory limit was set with the limits
// off, when nothing would apply it.
type LimitsOffError struct {
	Variable string
}

func (e *LimitsOffError) Error() string {
	return e.Variable + " applies only with DEMI_MANAGED_LIMITS=on"
}

// setting is one flag with the variable that sets it, its default and its
// parser. A setting is set by its flag, else by its variable, else it takes its
// default.
type setting struct {
	flag     string
	variable string
	fallback string
	// required means there is no default and the manager cannot start without
	// it.
	required bool
	// list means the flag may repeat and its value is a comma-separated list.
	list  bool
	parse func(value string) error
	// explicit is whether the flag or the variable set it.
	explicit bool
}

// Parse reads the configuration from args (the program's arguments after its
// name) and the variables lookup finds. An error names the variable, or the
// flag, of the setting it refuses. help receives the usage text of -h and
// --help.
func Parse(args []string, environ []string, help io.Writer) (*Config, error) {
	lookup := environment(environ)
	var config Config
	var recoverFlag, namespaceFlag, limitsOn bool
	var cpus, memory uint32
	limitsOn = true
	dns := []netip.Addr{}
	settings := []*setting{
		{flag: "socket", variable: "DEMI_MACHINES_SOCKET", parse: absolute(&config.Socket)},
		{flag: "data", variable: "DEMI_MACHINES_DATA", fallback: "/var/lib/demi-machines", parse: absolute(&config.Data)},
		{flag: "runsc", variable: "DEMI_MANAGED_RUNSC", required: true, parse: absolute(&config.Runsc)},
		{flag: "image", variable: "DEMI_MANAGED_IMAGE", required: true, parse: absolute(&config.Image)},
		{flag: "backend-url", variable: "DEMI_MANAGED_BACKEND_URL", required: true, parse: backendURL(&config.BackendURL)},
		{flag: "limits", variable: "DEMI_MANAGED_LIMITS", fallback: "on", parse: switchValue(&limitsOn)},
		{flag: "cpus", variable: "DEMI_MANAGED_CPUS", fallback: "2", parse: positive(&cpus)},
		{flag: "mem-mib", variable: "DEMI_MANAGED_MEM_MIB", fallback: "2048", parse: positive(&memory)},
		{flag: "system-mib", variable: "DEMI_MANAGED_SYSTEM_MIB", fallback: "1024", parse: positive(&config.SystemMiB)},
		{flag: "home-mib", variable: "DEMI_MANAGED_HOME_MIB", fallback: "1024", parse: positive(&config.HomeMiB)},
		{flag: "subnet", variable: "DEMI_MANAGED_SUBNET", fallback: "172.30.0.0/16", parse: subnet(&config.Subnet)},
		{flag: "slots", variable: "DEMI_MANAGED_SLOTS", fallback: "256", parse: slots(&config.Slots)},
		{flag: "dns", variable: "DEMI_MANAGED_DNS", required: true, list: true, parse: resolvers(&dns)},
	}
	if err := rejectUnknown(settings, environ); err != nil {
		return nil, err
	}
	commandLine := flag.NewFlagSet("demi-machines", flag.ContinueOnError)
	commandLine.SetOutput(io.Discard)
	// A flag given twice is refused, as clap refuses it, except the list.
	repeated := ""
	once := func(name string, seen *bool) {
		commandLine.BoolFunc(name, "", func(string) error {
			if *seen {
				repeated = name
			}
			*seen = true
			return nil
		})
	}
	once("recover", &recoverFlag)
	once("recover-namespace", &namespaceFlag)
	given := map[string][]string{}
	for _, s := range settings {
		commandLine.Func(s.flag, s.variable, func(value string) error {
			if len(given[s.flag]) > 0 && !s.list {
				repeated = s.flag
			}
			given[s.flag] = append(given[s.flag], value)
			return nil
		})
	}
	if err := commandLine.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			usage(help, commandLine, settings)
		}
		return nil, &UsageError{Err: err}
	}
	if repeated != "" {
		return nil, &UsageError{Err: fmt.Errorf("--%s cannot be used multiple times", repeated)}
	}
	if commandLine.NArg() > 0 {
		return nil, &UsageError{Err: fmt.Errorf("unexpected argument %q", commandLine.Arg(0))}
	}
	if recoverFlag && namespaceFlag {
		return nil, &UsageError{Err: errors.New("--recover and --recover-namespace cannot be used together")}
	}
	for _, s := range settings {
		if err := s.read(given[s.flag], lookup); err != nil {
			return nil, &UsageError{Err: err}
		}
	}
	capacity := uint64(1) << (32 - config.Subnet.Bits())
	if uint64(config.Slots)*4 > capacity {
		return nil, ErrSlotsExceedSubnet
	}
	switch {
	case recoverFlag:
		config.Mode = Recover
	case namespaceFlag:
		config.Mode = RecoverNamespace
	}
	if config.Mode == Serve && config.Socket == "" {
		return nil, ErrMissingSocket
	}
	if limitsOn {
		config.Limits = &Limits{CPUs: cpus, MemoryMiB: memory}
	} else {
		// A budget nothing would apply is refused, not ignored.
		for _, s := range settings {
			if (s.flag == "cpus" || s.flag == "mem-mib") && s.explicit {
				return nil, &LimitsOffError{Variable: s.variable}
			}
		}
	}
	config.DNS = dns
	return &config, nil
}

// read sets the setting from its flag values, else its variable, else its
// default.
func (s *setting) read(values []string, lookup func(string) (string, bool)) error {
	source := "--" + s.flag
	if len(values) == 0 {
		if value, ok := lookup(s.variable); ok {
			values = []string{value}
			source = s.variable
		}
	}
	if len(values) == 0 {
		if s.required {
			return fmt.Errorf("%s is required (--%s)", s.variable, s.flag)
		}
		if s.fallback == "" {
			return nil
		}
		values = []string{s.fallback}
	} else {
		s.explicit = true
	}
	if s.list {
		var items []string
		for _, value := range values {
			items = append(items, strings.Split(value, ",")...)
		}
		values = items
	}
	for _, value := range values {
		if err := s.parse(value); err != nil {
			return fmt.Errorf("invalid value for %s: %w", source, err)
		}
	}
	return nil
}

// environment returns the lookup of a list of NAME=VALUE entries.
func environment(environ []string) func(string) (string, bool) {
	values := make(map[string]string, len(environ))
	for _, entry := range environ {
		if name, value, ok := strings.Cut(entry, "="); ok {
			values[name] = value
		}
	}
	return func(name string) (string, bool) {
		value, ok := values[name]
		return value, ok
	}
}

// rejectUnknown refuses a DEMI_MANAGED_* variable the settings do not declare.
// The declared names come from the settings themselves, so they are listed once.
func rejectUnknown(settings []*setting, environ []string) error {
	var known []string
	for _, s := range settings {
		known = append(known, s.variable)
	}
	for _, entry := range environ {
		name, _, _ := strings.Cut(entry, "=")
		if strings.HasPrefix(name, managedPrefix) && !slices.Contains(known, name) {
			return &UnknownError{Name: name}
		}
	}
	return nil
}

func usage(out io.Writer, commandLine *flag.FlagSet, settings []*setting) {
	fmt.Fprintln(out, "The Cloud machine manager: runs users' Cloud machines as gVisor sandboxes and serves the backend over a Unix socket.")
	fmt.Fprintln(out, "\nUsage: demi-machines [--recover]")
	fmt.Fprintln(out, "\nSettings, each a flag or the variable named:")
	for _, s := range settings {
		fmt.Fprintf(out, "  --%s <%s>\n", s.flag, s.variable)
	}
	fmt.Fprintln(out, "\nFlags:")
	fmt.Fprintln(out, "  --recover  fence and save what a stopped manager left behind, then exit")
}

func absolute(target *string) func(string) error {
	return func(value string) error {
		if !filepath.IsAbs(value) {
			return errors.New("must be an absolute path")
		}
		*target = value
		return nil
	}
}

func backendURL(target **url.URL) func(string) error {
	return func(value string) error {
		// The URL parser reads a bare "#" as no fragment at all.
		if strings.Contains(value, "#") {
			return errors.New("must not have a fragment")
		}
		address, err := runnerproto.NormalURL(value)
		if err != nil {
			return err
		}
		if address.Scheme != "http" && address.Scheme != "https" {
			return errors.New("must be an http or https URL")
		}
		if address.Hostname() == "" {
			return errors.New("empty host")
		}
		*target = address
		return nil
	}
}

func switchValue(target *bool) func(string) error {
	return func(value string) error {
		switch value {
		case "on":
			*target = true
		case "off":
			*target = false
		default:
			return errors.New("must be on or off")
		}
		return nil
	}
}

// decimal parses a positive count or size written in decimal digits only: 0x10,
// 1e3, +12 and " 12 " are refused.
func decimal(value string, bits int) (uint64, error) {
	if value == "" || strings.Trim(value, "0123456789") != "" {
		return 0, errors.New("must be a positive decimal integer")
	}
	number, err := strconv.ParseUint(value, 10, bits)
	if err != nil {
		return 0, errors.New("number too large to fit in target type")
	}
	if number == 0 {
		return 0, errors.New("number would be zero for non-zero type")
	}
	return number, nil
}

func positive(target *uint32) func(string) error {
	return func(value string) error {
		number, err := decimal(value, 32)
		if err != nil {
			return err
		}
		*target = uint32(number)
		return nil
	}
}

func subnet(target *netip.Prefix) func(string) error {
	return func(value string) error {
		prefix, err := netip.ParsePrefix(value)
		if err != nil || !prefix.Addr().Is4() {
			return errors.New("invalid IPv4 network address")
		}
		if prefix.Bits() < 8 || prefix.Bits() > 30 || prefix.Addr() != prefix.Masked().Addr() {
			return errors.New("must be an aligned IPv4 network with prefix /8 through /30")
		}
		*target = prefix
		return nil
	}
}

func slots(target *uint16) func(string) error {
	return func(value string) error {
		number, err := decimal(value, 16)
		if err != nil {
			return err
		}
		if number < 1 || number > 16384 {
			return errors.New("must be from 1 to 16384")
		}
		*target = uint16(number)
		return nil
	}
}

// resolvers appends each resolver a sandbox can reach: not unspecified
// (0.0.0.0/8), loopback, multicast or broadcast.
func resolvers(target *[]netip.Addr) func(string) error {
	return func(value string) error {
		address, err := netip.ParseAddr(value)
		if err != nil || !address.Is4() {
			return errors.New("invalid IPv4 address")
		}
		if address.As4()[0] == 0 || address.IsLoopback() || address.IsMulticast() || address == netip.AddrFrom4([4]byte{255, 255, 255, 255}) {
			return errors.New("resolver must be reachable IPv4")
		}
		*target = append(*target, address)
		return nil
	}
}
