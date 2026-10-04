//go:build linux

package sandbox

import (
	_ "embed"
	"reflect"
	"testing"
)

//go:embed testdata/oci-config.json
var ociFixture []byte

func TestShippedOCIProfile(t *testing.T) {
	id := ID("demi-00000000-0000-4000-8000-000000000000")
	boot := Boot{
		Directory: NewRuntimeDirectory("/run/demi-machine-manager", id),
		Namespace: "demi-3",
		Cgroup:    &Cgroup{Name: id, Limits: Limits{CPUs: 2, MemoryMiB: 2048}},
	}
	data, err := Spec(boot)
	if err != nil {
		t.Fatal(err)
	}
	got, err := decodeOciSpec(data)
	if err != nil {
		t.Fatal(err)
	}
	expected, err := decodeOciSpec(ociFixture)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, expected) {
		t.Fatalf("profile differs from shipped fixture:\n%s", data)
	}
	boot.Cgroup = nil
	data, err = Spec(boot)
	if err != nil {
		t.Fatal(err)
	}
	got, err = decodeOciSpec(data)
	if err != nil {
		t.Fatal(err)
	}
	expected.Linux.CgroupsPath = nil
	expected.Linux.Resources = nil
	if !reflect.DeepEqual(got, expected) {
		t.Fatalf("limits-off profile differs:\n%s", data)
	}
}
