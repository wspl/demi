package machinesproto

import (
	"os"
	"regexp"
	"strconv"
	"testing"
)

// runnerWire is the Rust runner protocol's version constant, which the manager
// copies until the runner protocol moves to Go (runnerproto).
const runnerWire = "../../crates/runner-protocol/src/wire/mod.rs"

func TestTheRunnerWireVersionIsTheRusts(t *testing.T) {
	source, err := os.ReadFile(runnerWire)
	if err != nil {
		t.Fatal(err)
	}
	match := regexp.MustCompile(`pub const VERSION: u32 = (\d+);`).FindSubmatch(source)
	if match == nil {
		t.Fatalf("%s declares no VERSION", runnerWire)
	}
	version, err := strconv.Atoi(string(match[1]))
	if err != nil || version != runnerWireVersion {
		t.Errorf("the Rust runner wire is version %s, the manifest check has %d", match[1], runnerWireVersion)
	}
}
