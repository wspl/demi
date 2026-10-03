package machines_test

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/machines"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(programTests{m}) }

type programTests struct{ m *testing.M }

func (p programTests) Run() int { return programtest.Run(p.m) }

func configEnv(extra ...string) []string {
	return append(
		[]string{
			"DEMI_MANAGED_RUNSC=/opt/gvisor/runsc",
			"DEMI_MANAGED_IMAGE=/opt/image",
			"DEMI_MANAGED_BACKEND_URL=https://backend.example.com",
			"DEMI_MANAGED_DNS=1.1.1.1,8.8.8.8",
			"DEMI_MACHINE_MANAGER_SOCKET=/run/demi-cloud/machines.sock",
		},
		extra...)
}

func TestConfigDefaults(t *testing.T) {
	c, _, err := machines.ParseConfig(nil, configEnv())
	if err != nil {
		t.Fatal(err)
	}
	if c.Mode != machines.ModeServe || c.BackendURL.String() != "https://backend.example.com/" ||
		c.Data != "/var/lib/demi-machine-manager" ||
		c.Working() != "/var/lib/demi-machine-manager/working" ||
		c.Limits.CPUs != 2 ||
		c.Limits.MemoryBytes() != 2<<30 ||
		c.SystemBytes() != 1<<30 ||
		c.HomeBytes() != 1<<30 ||
		c.Subnet.String() != "172.30.0.0/16" ||
		c.Slots != 256 ||
		len(c.DNS) != 2 ||
		c.DNS[0].String() != "1.1.1.1" ||
		c.DNS[1].String() != "8.8.8.8" {
		t.Fatalf("defaults: %+v", c)
	}
}

func TestConfigRefusesInvalidSettings(t *testing.T) {
	cases := [][]string{
		{"DEMI_MANAGED_FIRECRACKER=/old"},
		{"DEMI_MANAGED_SUBNET=172.30.1.0/16"},
		{"DEMI_MANAGED_SUBNET=172.30.0.0/31"},
		{"DEMI_MANAGED_SUBNET=10.0.0.0/7"},
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
		{"DEMI_MACHINE_MANAGER_DATA=state"},
		{"DEMI_MANAGED_BACKEND_URL=file:///tmp/backend"},
		{"DEMI_MANAGED_LIMITS=yes"},
	}
	for _, settings := range cases {
		t.Run(strings.Join(settings, ","), func(t *testing.T) {
			if _, _, err := machines.ParseConfig(nil, configEnv(settings...)); err == nil {
				t.Fatal("invalid setting accepted")
			}
		})
	}
}

func TestLimitsOffRefusesExplicitBudgets(t *testing.T) {
	c, _, err := machines.ParseConfig(nil, configEnv("DEMI_MANAGED_LIMITS=off"))
	if err != nil || c.Limits != nil {
		t.Fatalf("limits off: %+v %v", c, err)
	}
	for _, setting := range []string{"DEMI_MANAGED_CPUS=2", "DEMI_MANAGED_MEM_MIB=2048"} {
		_, _, err := machines.ParseConfig(nil, configEnv("DEMI_MANAGED_LIMITS=off", setting))
		name, _, _ := strings.Cut(setting, "=")
		if err == nil || err.Error() != name+" applies only with DEMI_MANAGED_LIMITS=on" {
			t.Fatalf("budget: %v", err)
		}
	}
}

func TestUnknownManagedVariableNamed(t *testing.T) {
	_, _, err := machines.ParseConfig(nil, configEnv("DEMI_MANAGED_FIRECRACKER=/old"))
	if err == nil || err.Error() != "DEMI_MANAGED_FIRECRACKER is not a Cloud manager setting" {
		t.Fatal(err)
	}
}

func TestServingNeedsSocketRecoveryDoesNot(t *testing.T) {
	env := configEnv()
	env = env[:len(env)-1]
	c, _, err := machines.ParseConfig([]string{"--recover"}, env)
	if err != nil || c.Mode != machines.ModeRecover {
		t.Fatalf("recovery: %+v %v", c, err)
	}
	// A missing socket is reported with this exact text.
	if _, _, err = machines.ParseConfig(
		nil,
		env,
	); err == nil ||
		err.Error() != "DEMI_MACHINE_MANAGER_SOCKET is required" {
		t.Fatal(err)
	}
	if _, _, err = machines.ParseConfig([]string{"--recover", "--recover-namespace"}, env); err == nil {
		t.Fatal("conflicting modes accepted")
	}
}

// These admission scenarios use virtual scheduling and take no wall-clock waits.
func TestExclusiveAdmissionHoldsBackLaterEntrants(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var a machines.Admission
		release, err := a.Enter(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		order := make(chan string, 3)
		finish := make(chan struct{})
		done := make(chan struct{})
		go func() {
			defer close(done)
			exclusive, err := a.Exclusive(t.Context())
			if err != nil {
				t.Error(err)
				return
			}
			defer exclusive()
			order <- "exclusive"
			<-finish
			order <- "exclusive done"
		}()
		synctest.Wait()
		laterDone := make(chan struct{})
		go func() {
			defer close(laterDone)
			later, err := a.Enter(t.Context())
			if err != nil {
				t.Error(err)
				return
			}
			defer later()
			order <- "later"
		}()
		synctest.Wait()
		if len(order) != 0 {
			t.Fatal("exclusive passed shared work")
		}
		release()
		synctest.Wait()
		if len(order) != 1 {
			t.Fatal("later passed exclusive")
		}
		close(finish)
		<-done
		<-laterDone
		got := []string{<-order, <-order, <-order}
		if !reflect.DeepEqual(got, []string{"exclusive", "exclusive done", "later"}) {
			t.Fatal(got)
		}
	})
}

func TestClosedAdmissionRefusesWaitingAndNewWork(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var a machines.Admission
		whole, err := a.Exclusive(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		result := make(chan error, 1)
		go func() {
			release, err := a.Enter(t.Context())
			if release != nil {
				release()
			}
			result <- err
		}()
		synctest.Wait()
		a.Close()
		whole()
		if err := <-result; !errors.Is(err, machines.ErrClosed) {
			t.Fatal(err)
		}
		if _, err = a.Enter(context.Background()); !errors.Is(err, machines.ErrClosed) {
			t.Fatal(err)
		}
	})
}
