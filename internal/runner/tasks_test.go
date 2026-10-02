package runner

import (
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/runnerwire"
)

// Environment scenarios run entirely in memory.
func TestJobEnvironmentCombinesDeviceRequestAndOwnedValues(t *testing.T) {
	environment := taskEnvironment{values: map[string]string{"DEVICE": "device", "OVERRIDE": "old"}, home: "/home/user", installation: "/private/runner"}
	spec := environment.shellSpec(&runnerwire.JobStart{JobID: "env", CWD: "/work", Env: map[string]string{"OVERRIDE": "new", "DEMI_HOME": "untrusted"}})
	want := map[string]string{"DEVICE": "device", "OVERRIDE": "new", "HOME": "/home/user", "DEMI_HOME": "/private/runner"}
	if diff := cmp.Diff(want, spec.Env); diff != "" {
		t.Fatal(diff)
	}
	if environment.values["OVERRIDE"] != "old" {
		t.Fatal("request changed the device environment")
	}
}

func TestRawSpawnInheritsEnvironmentOnlyWhenRequested(t *testing.T) {
	caller := "caller"
	yes := true
	for _, test := range []struct {
		name    string
		inherit *bool
		env     *map[string]*string
		want    map[string]string
	}{
		{"default", nil, nil, map[string]string{"DEVICE": "device", "OVERRIDE": "device", "DEMI_HOME": "/runner"}},
		{"replace", nil, &map[string]*string{"OVERRIDE": &caller}, map[string]string{"OVERRIDE": "caller", "DEMI_HOME": "/runner"}},
		{"extend", &yes, &map[string]*string{"OVERRIDE": &caller}, map[string]string{"DEVICE": "device", "OVERRIDE": "caller", "DEMI_HOME": "/runner"}},
		{"remove", &yes, &map[string]*string{"DEVICE": nil}, map[string]string{"OVERRIDE": "device", "DEMI_HOME": "/runner"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			environment := taskEnvironment{values: map[string]string{"DEVICE": "device", "OVERRIDE": "device"}, installation: "/runner"}
			got := environment.processSpec(&runnerwire.Spawn{Env: test.env, InheritEnv: test.inherit})
			if diff := cmp.Diff(test.want, got.Env); diff != "" {
				t.Fatal(diff)
			}
		})
	}
}
