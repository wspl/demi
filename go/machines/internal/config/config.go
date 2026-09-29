// Package config reads the machine manager's configuration
// (docs/cloud/setup.md § Configuration): its command line and the
// DEMI_MACHINES_* and DEMI_MANAGED_* variables, validated at startup. An invalid
// or unknown setting stops the manager with an error that names the variable.
package config

import (
	"errors"
	"fmt"
	"io"
	"net/netip"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/go/internal/envflag"
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
	BackendURL *whatwg.Url
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

// Parse reads the configuration from args (the program's arguments after its
// name) and the variables lookup finds. An error names the variable, or the
// flag, of the setting it refuses; an envflag.UsageError is one clap would
// report. help receives the usage text of -h and --help.
func Parse(args []string, environ []string, help io.Writer) (*Config, error) {
	var config Config
	var recoverFlag, namespaceFlag, limitsOn bool
	var cpus, memory uint32
	limitsOn = true
	dns := []netip.Addr{}
	cpusSetting := &envflag.Setting{Flag: "cpus", Variable: "DEMI_MANAGED_CPUS", Fallback: "2", Parse: positive(&cpus)}
	memorySetting := &envflag.Setting{Flag: "mem-mib", Variable: "DEMI_MANAGED_MEM_MIB", Fallback: "2048", Parse: positive(&memory)}
	settings := []*envflag.Setting{
		{Flag: "socket", Variable: "DEMI_MACHINES_SOCKET", Parse: absolute(&config.Socket)},
		{Flag: "data", Variable: "DEMI_MACHINES_DATA", Fallback: "/var/lib/demi-machines", Parse: absolute(&config.Data)},
		{Flag: "runsc", Variable: "DEMI_MANAGED_RUNSC", Required: true, Parse: absolute(&config.Runsc)},
		{Flag: "image", Variable: "DEMI_MANAGED_IMAGE", Required: true, Parse: absolute(&config.Image)},
		{Flag: "backend-url", Variable: "DEMI_MANAGED_BACKEND_URL", Required: true, Parse: backendURL(&config.BackendURL)},
		{Flag: "limits", Variable: "DEMI_MANAGED_LIMITS", Fallback: "on", Parse: switchValue(&limitsOn)},
		cpusSetting,
		memorySetting,
		{Flag: "system-mib", Variable: "DEMI_MANAGED_SYSTEM_MIB", Fallback: "1024", Parse: positive(&config.SystemMiB)},
		{Flag: "home-mib", Variable: "DEMI_MANAGED_HOME_MIB", Fallback: "1024", Parse: positive(&config.HomeMiB)},
		{Flag: "subnet", Variable: "DEMI_MANAGED_SUBNET", Fallback: "172.30.0.0/16", Parse: subnet(&config.Subnet)},
		{Flag: "slots", Variable: "DEMI_MANAGED_SLOTS", Fallback: "256", Parse: slots(&config.Slots)},
		{Flag: "dns", Variable: "DEMI_MANAGED_DNS", Required: true, List: true, Parse: resolvers(&dns)},
	}
	if err := rejectUnknown(settings, environ); err != nil {
		return nil, err
	}
	command := envflag.Command{
		Name:     "demi-machines",
		Settings: settings,
		Switches: []envflag.Switch{{Flag: "recover", Target: &recoverFlag}, {Flag: "recover-namespace", Target: &namespaceFlag}},
		Usage:    func(out io.Writer) { usage(out, settings) },
	}
	if err := command.Parse(args, envflag.Environment(environ), help); err != nil {
		return nil, err
	}
	if recoverFlag && namespaceFlag {
		return nil, &envflag.UsageError{Err: errors.New("--recover and --recover-namespace cannot be used together")}
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
		for _, s := range []*envflag.Setting{cpusSetting, memorySetting} {
			if s.Explicit() {
				return nil, &LimitsOffError{Variable: s.Variable}
			}
		}
	}
	config.DNS = dns
	return &config, nil
}

// rejectUnknown refuses a DEMI_MANAGED_* variable the settings do not declare.
// The declared names come from the settings themselves, so they are listed once.
func rejectUnknown(settings []*envflag.Setting, environ []string) error {
	var known []string
	for _, s := range settings {
		known = append(known, s.Variable)
	}
	for _, entry := range environ {
		name, _, _ := strings.Cut(entry, "=")
		if strings.HasPrefix(name, managedPrefix) && !slices.Contains(known, name) {
			return &UnknownError{Name: name}
		}
	}
	return nil
}

func usage(out io.Writer, settings []*envflag.Setting) {
	fmt.Fprintln(out, "The Cloud machine manager: runs users' Cloud machines as gVisor sandboxes and serves the backend over a Unix socket.")
	fmt.Fprintln(out, "\nUsage: demi-machines [--recover]")
	fmt.Fprintln(out, "\nSettings, each a flag or the variable named:")
	for _, s := range settings {
		fmt.Fprintf(out, "  --%s <%s>\n", s.Flag, s.Variable)
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

func backendURL(target **whatwg.Url) func(string) error {
	return func(value string) error {
		// The URL parser reads a bare "#" as no fragment at all.
		if strings.Contains(value, "#") {
			return errors.New("must not have a fragment")
		}
		address, err := runnerproto.NormalURL(value)
		if err != nil {
			return err
		}
		if address.Scheme() != "http" && address.Scheme() != "https" {
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
