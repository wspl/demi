package config_test

import (
	"errors"
	"io"
	"net/netip"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/internal/envflag"
	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/runnerproto"
)

// required are the settings a manager cannot start without.
var required = []string{
	"DEMI_MANAGED_RUNSC=/opt/gvisor/runsc",
	"DEMI_MANAGED_IMAGE=/opt/image",
	"DEMI_MANAGED_BACKEND_URL=https://backend.example.com",
	"DEMI_MANAGED_DNS=1.1.1.1,8.8.8.8",
	"DEMI_MACHINES_SOCKET=/run/demi-cloud/machines.sock",
}

// parse reads the required variables with settings over them.
func parse(args []string, settings ...string) (*config.Config, error) {
	environ := slices.Clone(required)
	for _, setting := range settings {
		name := setting[:slices.Index([]byte(setting), '=')]
		environ = slices.DeleteFunc(environ, func(entry string) bool { return len(entry) > len(name) && entry[:len(name)+1] == name+"=" })
		environ = append(environ, setting)
	}
	return config.Parse(args, environ, io.Discard)
}

// The configured backend is written as a boot file writes it, so a sandbox
// whose boot names the configured backend is admitted, whatever the path holds.
// Cost: parsing only.
func TestTheBackendURLIsWrittenAsBootFilesWriteIt(t *testing.T) {
	for _, raw := range []string{"https://Backend.example.com:443/a|b", "https://backend.example.com/%zz^"} {
		cfg, err := parse(nil, "DEMI_MANAGED_BACKEND_URL="+raw)
		if err != nil {
			t.Fatal(err)
		}
		boot, err := runnerproto.BackendURL(raw).Normal()
		if err != nil || cfg.BackendURL.String() != boot {
			t.Errorf("%s: configured %s, boot %s %v", raw, cfg.BackendURL, boot, err)
		}
	}
}

func TestDefaultsApplyAndTheBackendURLIsNormalized(t *testing.T) {
	cfg, err := parse(nil)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Mode != config.Serve || cfg.BackendURL.String() != "https://backend.example.com/" {
		t.Errorf("mode %v, backend %s", cfg.Mode, cfg.BackendURL)
	}
	if cfg.Data != "/var/lib/demi-machines" || cfg.Working() != "/var/lib/demi-machines/working" || cfg.Images() != "/var/lib/demi-machines/images" {
		t.Errorf("data %s, working %s", cfg.Data, cfg.Working())
	}
	if cfg.Limits == nil || cfg.Limits.CPUs != 2 || cfg.Limits.MemoryBytes() != 2<<30 {
		t.Errorf("limits %+v", cfg.Limits)
	}
	if cfg.SystemBytes() != 1<<30 || cfg.HomeBytes() != 1<<30 {
		t.Errorf("capacities %d, %d", cfg.SystemBytes(), cfg.HomeBytes())
	}
	if cfg.Subnet.String() != "172.30.0.0/16" || cfg.Slots != 256 {
		t.Errorf("pool %s with %d slots", cfg.Subnet, cfg.Slots)
	}
	dns := []netip.Addr{netip.MustParseAddr("1.1.1.1"), netip.MustParseAddr("8.8.8.8")}
	if !slices.Equal(cfg.DNS, dns) {
		t.Errorf("dns %v", cfg.DNS)
	}
	if config.RuntimeDirectory != "/run/demi-machines" || cfg.Runtime() != "/run/demi-machines" {
		t.Errorf("runtime directory %s", cfg.Runtime())
	}
}

func TestObsoleteMalformedAndInsufficientSettingsAreRefused(t *testing.T) {
	for _, settings := range [][]string{
		{"DEMI_MANAGED_FIRECRACKER=/old"},
		{"DEMI_MANAGED_SUBNET=172.30.1.0/16"},
		{"DEMI_MANAGED_SUBNET=172.30.0.0/31"},
		{"DEMI_MANAGED_SUBNET=10.0.0.0/7"},
		// Two slots need eight addresses; a /30 holds four.
		{"DEMI_MANAGED_SUBNET=172.30.0.0/30", "DEMI_MANAGED_SLOTS=2"},
		{"DEMI_MANAGED_DNS=127.0.0.1"},
		{"DEMI_MANAGED_DNS=0.1.2.3"},
		{"DEMI_MANAGED_DNS=224.0.0.1"},
		{"DEMI_MANAGED_DNS=255.255.255.255"},
		{"DEMI_MANAGED_DNS=1.1.1.1,"},
		{"DEMI_MANAGED_CPUS=0"},
		{"DEMI_MANAGED_CPUS=0x10"},
		{"DEMI_MANAGED_MEM_MIB=1e3"},
		{"DEMI_MANAGED_SYSTEM_MIB= 12 "},
		{"DEMI_MANAGED_HOME_MIB=+12"},
		{"DEMI_MANAGED_SLOTS=16385"},
		{"DEMI_MANAGED_RUNSC=runsc"},
		{"DEMI_MACHINES_DATA=state"},
		{"DEMI_MANAGED_BACKEND_URL=file:///tmp/backend"},
		{"DEMI_MANAGED_BACKEND_URL=http://"},
		// What the URL parser of the Rust refuses or the manager refuses later.
		{"DEMI_MANAGED_BACKEND_URL=https://backend.example.com/#"},
		{"DEMI_MANAGED_BACKEND_URL=https://backend.example.com/#top"},
		{"DEMI_MANAGED_BACKEND_URL=https://backend.example.com:65536"},
		{"DEMI_MANAGED_LIMITS=yes"},
	} {
		if _, err := parse(nil, settings...); err == nil {
			t.Errorf("%v was accepted", settings)
		}
	}
}

func TestAFlagSetsWhatItsVariableSetsAndWinsOverIt(t *testing.T) {
	cfg, err := parse([]string{"--slots=8", "--dns", "9.9.9.9", "--dns=1.0.0.1"}, "DEMI_MANAGED_SLOTS=4")
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Slots != 8 || !slices.Equal(cfg.DNS, []netip.Addr{netip.MustParseAddr("9.9.9.9"), netip.MustParseAddr("1.0.0.1")}) {
		t.Errorf("slots %d, dns %v", cfg.Slots, cfg.DNS)
	}
}

func TestWithTheLimitsOffSandboxesHaveNoneAndABudgetIsRefused(t *testing.T) {
	cfg, err := parse(nil, "DEMI_MANAGED_LIMITS=off")
	if err != nil || cfg.Limits != nil {
		t.Fatalf("the limits off: %+v, %v", cfg, err)
	}
	for variable, value := range map[string]string{"DEMI_MANAGED_CPUS": "2", "DEMI_MANAGED_MEM_MIB": "2048"} {
		// The default value, written out, is still a budget nothing applies.
		_, err := parse(nil, "DEMI_MANAGED_LIMITS=off", variable+"="+value)
		var refused *config.LimitsOffError
		if !errors.As(err, &refused) || err.Error() != variable+" applies only with DEMI_MANAGED_LIMITS=on" {
			t.Errorf("%s: %v", variable, err)
		}
	}
}

func TestAnUnknownManagedVariableIsNamed(t *testing.T) {
	_, err := parse(nil, "DEMI_MANAGED_FIRECRACKER=/old")
	if err == nil || err.Error() != "DEMI_MANAGED_FIRECRACKER is not a Cloud manager setting" {
		t.Errorf("%v", err)
	}
}

func TestServingNeedsASocketAndRecoveryDoesNot(t *testing.T) {
	without := slices.DeleteFunc(slices.Clone(required), func(entry string) bool { return entry[:len("DEMI_MACHINES_SOCKET")] == "DEMI_MACHINES_SOCKET" })
	recovering, err := config.Parse([]string{"--recover"}, without, io.Discard)
	if err != nil || recovering.Mode != config.Recover {
		t.Fatalf("recovery needs no socket: %+v, %v", recovering, err)
	}
	inside, err := config.Parse([]string{"--recover-namespace"}, without, io.Discard)
	if err != nil || inside.Mode != config.RecoverNamespace {
		t.Fatalf("recovery inside the namespace: %+v, %v", inside, err)
	}
	if _, err := config.Parse(nil, without, io.Discard); !errors.Is(err, config.ErrMissingSocket) {
		t.Errorf("serving without a socket: %v", err)
	}
	if _, err := parse([]string{"--recover", "--recover-namespace"}); err == nil {
		t.Error("the two recoveries were accepted together")
	}
}

func TestAFlagGivenTwiceIsRefusedExceptTheList(t *testing.T) {
	for _, args := range [][]string{{"--slots=8", "--slots=9"}, {"--recover", "--recover"}, {"--limits=off", "--limits=off"}} {
		if _, err := parse(args); err == nil || !envflag.IsUsage(err) {
			t.Errorf("%v: %v", args, err)
		}
	}
}

// The Rust exits with status 2 for what clap refuses (a flag, a value, a missing
// setting) and 1 for the manager's own refusals.
func TestUsageErrorsAreToldFromTheManagersOwnRefusals(t *testing.T) {
	for name, test := range map[string]struct {
		args     []string
		settings []string
		usage    bool
	}{
		"a value that does not parse":  {nil, []string{"DEMI_MANAGED_CPUS=0"}, true},
		"an unknown flag":              {[]string{"--nonsense"}, nil, true},
		"an unexpected argument":       {[]string{"extra"}, nil, true},
		"an unknown variable":          {nil, []string{"DEMI_MANAGED_FIRECRACKER=/old"}, false},
		"slots over the subnet":        {nil, []string{"DEMI_MANAGED_SUBNET=172.30.0.0/30", "DEMI_MANAGED_SLOTS=2"}, false},
		"a budget with the limits off": {nil, []string{"DEMI_MANAGED_LIMITS=off", "DEMI_MANAGED_CPUS=2"}, false},
	} {
		if _, err := parse(test.args, test.settings...); err == nil || envflag.IsUsage(err) != test.usage {
			t.Errorf("%s: %v, usage %v", name, err, envflag.IsUsage(err))
		}
	}
	if _, err := config.Parse(nil, slices.DeleteFunc(slices.Clone(required), func(entry string) bool { return strings.HasPrefix(entry, "DEMI_MACHINES_SOCKET") }), io.Discard); err == nil || envflag.IsUsage(err) {
		t.Errorf("a missing socket: %v", err)
	}
}
