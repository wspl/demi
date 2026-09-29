package main

import (
	"context"
	"errors"
	"log/slog"
	"math"
	"slices"
	"strings"
)

// levelTrace is tracing's trace, below slog's debug: what the Claude Code
// CLI's raw exchange is logged at (claude-code.md § Process lifetime).
const levelTrace = slog.LevelDebug - 4

// levelOff is above the level of every record.
const levelOff = slog.Level(math.MaxInt)

// logFilter is DEMI_LOG (backend.md § Configuration), in tracing-subscriber's
// Targets syntax: comma-separated directives, each a level, a target=level
// pair, or a bare target, which logs everything of it. A record's target is
// the "target" attribute of the logger that wrote it, and a directive covers
// each target it is a prefix of; the longest one that covers it decides.
// The bare level covers every record, a record without a target among them,
// and without one nothing is logged of what no directive covers.
type logFilter struct {
	fallback slog.Level
	// targets are longest first.
	targets []targetLevel
}

type targetLevel struct {
	target string
	level  slog.Level
}

func parseLogFilter(text string) (logFilter, error) {
	filter := logFilter{fallback: levelOff}
	for _, directive := range strings.Split(text, ",") {
		target, levelText, pair := strings.Cut(directive, "=")
		if pair {
			if strings.Contains(levelText, "=") {
				return logFilter{}, errors.New("too many '=' in filter directive, expected 0 or 1")
			}
			// tracing's span field lists name no target in Go.
			if strings.Contains(target, "[") {
				return logFilter{}, errors.New("a directive names a target, without span fields")
			}
			level, err := parseLevel(levelText)
			if err != nil {
				return logFilter{}, err
			}
			filter.targets = append(filter.targets, targetLevel{target, level})
			continue
		}
		if level, err := parseLevel(directive); err == nil {
			filter.fallback = level
			continue
		}
		filter.targets = append(filter.targets, targetLevel{directive, levelTrace})
	}
	slices.SortStableFunc(filter.targets, func(a, b targetLevel) int { return len(b.target) - len(a.target) })
	return filter, nil
}

// parseLevel reads a level as tracing does: a name in any case, or a number
// from 0 (off) to 5 (trace).
func parseLevel(text string) (slog.Level, error) {
	switch strings.ToLower(text) {
	case "off", "0":
		return levelOff, nil
	case "error", "1":
		return slog.LevelError, nil
	case "warn", "2":
		return slog.LevelWarn, nil
	case "info", "3":
		return slog.LevelInfo, nil
	case "debug", "4":
		return slog.LevelDebug, nil
	case "trace", "5":
		return levelTrace, nil
	}
	return 0, errors.New(`expected one of "off", "error", "warn", "info", "debug", "trace", or a number 0-5`)
}

// levelOf is the least level logged of target; "" for a record without one.
func (f logFilter) levelOf(target string) slog.Level {
	for _, directive := range f.targets {
		if strings.HasPrefix(target, directive.target) {
			return directive.level
		}
	}
	return f.fallback
}

// filtered passes to next the records its filter logs.
type filtered struct {
	filter logFilter
	// least is the least level logged of this handler's target.
	least slog.Level
	next  slog.Handler
}

func newFiltered(filter logFilter, next slog.Handler) *filtered {
	return &filtered{filter: filter, least: filter.levelOf(""), next: next}
}

func (h *filtered) Enabled(ctx context.Context, level slog.Level) bool {
	return level >= h.least && h.next.Enabled(ctx, level)
}

func (h *filtered) Handle(ctx context.Context, record slog.Record) error {
	return h.next.Handle(ctx, record)
}

func (h *filtered) WithAttrs(attrs []slog.Attr) slog.Handler {
	least := h.least
	for _, attr := range attrs {
		if attr.Key == "target" {
			least = h.filter.levelOf(attr.Value.String())
		}
	}
	return &filtered{filter: h.filter, least: least, next: h.next.WithAttrs(attrs)}
}

func (h *filtered) WithGroup(name string) slog.Handler {
	return &filtered{filter: h.filter, least: h.least, next: h.next.WithGroup(name)}
}
