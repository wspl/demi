package backend

import (
	"bytes"
	"log/slog"
	"strings"
	"testing"
	"time"
)

// TestLogTargets
// checks the documented log filter at the output boundary.
// Checks the configuration's documented filter at the output boundary, without
// process-global logger changes or wall-time waits.
func TestLogTargets(t *testing.T) {
	t.Parallel()
	for _, scenario := range []struct {
		filter  string
		level   slog.Level
		attrs   []slog.Attr
		written bool
	}{
		{"", slog.LevelInfo, nil, false},
		{"", slog.LevelError, nil, true},
		{"info,", slog.LevelInfo, nil, false},
		{"=debug,info", slog.LevelDebug, nil, false},
		{"=debug,info", slog.LevelDebug, []slog.Attr{slog.String("target", "")}, true},
		{"info,wire=trace", -8, []slog.Attr{slog.String("target", "wire::frames")}, true},
		{"info,wire=trace,wire=off", slog.LevelError, []slog.Attr{slog.String("target", "wire")}, false},
		{"info,wire[{raw}]=trace", -8, []slog.Attr{slog.String("target", "wire"), slog.String("raw", "frame")}, true},
		{"info,wire[{raw}]=trace", -8, []slog.Attr{slog.String("target", "wire")}, false},
		{"info, wire=trace", -8, []slog.Attr{slog.String("target", "wire")}, false},
	} {
		targets, err := parseLogTargets(scenario.filter)
		if err != nil {
			t.Fatal(err)
		}
		var output bytes.Buffer
		handler := &targetHandler{
			next:    slog.NewTextHandler(&output, &slog.HandlerOptions{Level: slog.Level(-8)}),
			targets: targets,
		}
		record := slog.NewRecord(time.Time{}, scenario.level, "fixture record", 0)
		record.AddAttrs(scenario.attrs...)
		if err := handler.Handle(t.Context(), record); err != nil {
			t.Fatal(err)
		}
		if strings.Contains(output.String(), "fixture record") != scenario.written {
			t.Fatalf("filter %q, level %s: %q", scenario.filter, scenario.level, output.String())
		}
	}
	for _, filter := range []string{"wire=nope", "wire=info=debug", "wire[{raw=trace"} {
		if _, err := parseLogTargets(filter); err == nil {
			t.Fatalf("accepted invalid filter %q", filter)
		}
	}
}
