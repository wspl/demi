package main

import (
	"log/slog"
	"testing"
)

// DEMI_LOG in tracing's Targets syntax: the bare level covers every record,
// and a target's directive covers the targets it is a prefix of, the longest
// deciding.
// Cost: parsing only.
func TestDEMILOGSetsADefaultLevelAndALevelPerTarget(t *testing.T) {
	filter, err := parseLogFilter("info,demi::provider::claude_code::wire=trace,demi::provider=WARN,demi::quiet")
	if err != nil {
		t.Fatal(err)
	}
	for target, want := range map[string]slog.Level{
		"":                                  slog.LevelInfo,
		"demi::edge":                        slog.LevelInfo,
		"demi::provider::codex":             slog.LevelWarn,
		"demi::provider::claude_code::wire": levelTrace,
		"demi::provider::claude_code::wire::frame": levelTrace,
		"demi::quiet::inner":                       levelTrace,
	} {
		if got := filter.levelOf(target); got != want {
			t.Errorf("%q logs from %v, want %v", target, got, want)
		}
	}
	only, err := parseLogFilter("demi::provider=5")
	if err != nil {
		t.Fatal(err)
	}
	if only.levelOf("") != levelOff || only.levelOf("demi::edge") != levelOff || only.levelOf("demi::provider") != levelTrace {
		t.Error("without a bare level, only the named target logs")
	}
	for _, text := range []string{"demi=loud", "demi=info=debug", "demi[{field}]=info"} {
		if _, err := parseLogFilter(text); err == nil {
			t.Errorf("%q was accepted", text)
		}
	}
}
