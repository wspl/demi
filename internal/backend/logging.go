package backend

import (
	"context"
	"errors"
	"log/slog"
	"slices"
	"strconv"
	"strings"
)

type logTarget struct {
	target *string
	fields []string
	level  slog.Level
}

// parseLogTargets reads the backend's tracing-subscriber Targets syntax.
func parseLogTargets(text string) ([]logTarget, error) {
	result := []logTarget{}
	levels := map[string]slog.Level{"": slog.LevelError, "off": 100, "error": slog.LevelError, "warn": slog.LevelWarn, "info": slog.LevelInfo, "debug": slog.LevelDebug, "trace": -8}
	for _, entry := range strings.Split(text, ",") {
		pieces := strings.Split(entry, "=")
		if len(pieces) > 2 {
			return nil, errors.New("invalid filter directive: too many '=' in filter directive, expected 0 or 1")
		}
		rule := logTarget{level: -8}
		level := pieces[len(pieces)-1]
		if number, err := strconv.ParseUint(strings.TrimPrefix(level, "+"), 10, 64); err == nil && number <= 5 {
			level = []string{"off", "error", "warn", "info", "debug", "trace"}[number]
		}
		parsed, ok := levels[strings.ToLower(level)]
		if len(pieces) == 1 {
			if ok {
				rule.level = parsed
			} else {
				rule.target = &entry
			}
		} else {
			if !ok {
				return nil, errors.New(`error parsing level filter: expected one of "off", "error", "warn", "info", "debug", "trace", or a number 0-5`)
			}
			rule.level = parsed
			target := strings.Split(pieces[0], "[{")
			if len(target) > 2 {
				return nil, errors.New("invalid filter directive: too many '[{' in filter directive, expected 0 or 1")
			}
			rule.target = &target[0]
			if len(target) == 2 {
				if !strings.HasSuffix(target[1], "}]") {
					return nil, errors.New("invalid filter directive: expected fields list to end with '}]'")
				}
				fields := target[1]
				for strings.HasSuffix(fields, "}]") {
					fields = strings.TrimSuffix(fields, "}]")
				}
				for _, field := range strings.Split(fields, ",") {
					if field != "" {
						rule.fields = append(rule.fields, field)
					}
				}
			}
		}
		result = append(result, rule)
	}
	return result, nil
}

type targetHandler struct {
	next    slog.Handler
	targets []logTarget
	attrs   []slog.Attr
}

func (*targetHandler) Enabled(context.Context, slog.Level) bool { return true }
func (h *targetHandler) Handle(ctx context.Context, record slog.Record) error {
	var target *string
	fields := make(map[string]bool)
	visit := func(a slog.Attr) bool {
		fields[a.Key] = true
		if a.Key == "target" {
			name := a.Value.String()
			target = &name
		}
		return true
	}
	for _, a := range h.attrs {
		visit(a)
	}
	record.Attrs(visit)
	threshold := slog.Level(100)
	var selected *logTarget
	for i := range h.targets {
		rule := &h.targets[i]
		if rule.target != nil && (target == nil || !strings.HasPrefix(*target, *rule.target)) {
			continue
		}
		matches := true
		for _, field := range rule.fields {
			matches = matches && fields[field]
		}
		if !matches {
			continue
		}
		rank, previous := -1, -1
		if rule.target != nil {
			rank = len(*rule.target)
		}
		if selected != nil && selected.target != nil {
			previous = len(*selected.target)
		}
		if selected == nil || rank > previous || rank == previous && (len(rule.fields) > len(selected.fields) || len(rule.fields) == len(selected.fields) && slices.Compare(rule.fields, selected.fields) >= 0) {
			selected = rule
			threshold = rule.level
		}
	}
	if record.Level < threshold {
		return nil
	}
	return h.next.Handle(ctx, record)
}
func (h *targetHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	return &targetHandler{next: h.next.WithAttrs(attrs), targets: h.targets, attrs: append(append([]slog.Attr{}, h.attrs...), attrs...)}
}
func (h *targetHandler) WithGroup(name string) slog.Handler {
	return &targetHandler{next: h.next.WithGroup(name), targets: h.targets, attrs: h.attrs}
}
