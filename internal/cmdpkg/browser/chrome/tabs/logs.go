package tabs

import (
	"context"
	"errors"
	"slices"
	"strings"
	"sync"

	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/contract"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Console reads the tab-owned bounded console history. The tab joins its collector.
type Console struct {
	ctx context.Context
	// mu protects the console history; it is never held across IO.
	mu      sync.Mutex
	history *cdp.Buffer[browserop.LogEntry]
}

// Read filters and pages console entries, retaining independent cursors and gaps.
func (c *Console) Read(ctx context.Context, input browserop.LogsInput) (browserop.LogsResult, error) {
	if err := ctx.Err(); err != nil {
		return browserop.LogsResult{}, err
	}
	if c.ctx.Err() != nil {
		return browserop.LogsResult{}, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	return readConsole(c.history, input)
}

// observeConsole subscribes before navigation and owns collection for the tab's lifetime.
func observeConsole(tab *Tab) (*Console, error) {
	events, err := tab.Subscribe("Runtime.consoleAPICalled")
	if err != nil {
		return nil, err
	}
	history, err := cdp.NewBuffer(
		"logs",
		browserop.ConsoleEntries,
		browserop.ConsoleBytes,
		func(entry browserop.LogEntry) uint64 { return entry.Sequence },
		func(entry browserop.LogEntry, sequence uint64) browserop.LogEntry {
			entry.Sequence = sequence
			return entry
		},
	)
	if err != nil {
		events.Close()
		return nil, err
	}
	console := &Console{ctx: tab.ctx, history: history}
	if err := tab.StartTask(func(ctx context.Context) {
		console.collect(ctx, events, history)
	}); err != nil {
		events.Close()
		return nil, err
	}
	return console, nil
}

// consoleEntry preserves Chrome's console values and their first call-frame URL.
func consoleEntry(event *runtime.EventConsoleAPICalled) (browserop.LogEntry, error) {
	parts := make([]string, 0, len(event.Args))
	for _, argument := range event.Args {
		text := argument.Description
		if argument.UnserializableValue != "" {
			text = string(argument.UnserializableValue)
		}
		var err error
		text, err = consoleValue(argument, text)
		if err != nil {
			return browserop.LogEntry{}, err
		}
		parts = append(parts, text)
	}
	level := browserop.LogLevelLog
	switch string(event.Type) {
	case "debug":
		level = browserop.LogLevelDebug
	case "info":
		level = browserop.LogLevelInfo
	case "warning":
		level = browserop.LogLevelWarning
	case "error", "assert":
		level = browserop.LogLevelError
	}
	var url *string
	if event.StackTrace != nil && len(event.StackTrace.CallFrames) > 0 {
		value := event.StackTrace.CallFrames[0].URL
		url = &value
	}
	return browserop.LogEntry{
		Level:     level,
		Text:      strings.Join(parts, " "),
		URL:       url,
		Timestamp: float64(event.Timestamp.Time().UnixNano()) / 1e6,
	}, nil
}

// readConsole pages a bounded history without consuming another reader's cursor.
func readConsole(history *cdp.Buffer[browserop.LogEntry], input browserop.LogsInput) (browserop.LogsResult, error) {
	var after uint64
	if input.After != nil {
		var err error
		after, err = history.Position(*input.After)
		if err != nil {
			return browserop.LogsResult{}, err
		}
	}
	entries := []browserop.LogEntry{}
	for _, entry := range history.Entries() {
		if input.After != nil && entry.Sequence < after {
			continue
		}
		if input.Level != nil && !slices.Contains(*input.Level, entry.Level) {
			continue
		}
		if input.Filter != nil && !strings.Contains(entry.Text, *input.Filter) {
			continue
		}
		entries = append(entries, entry)
	}
	limit := uint(browserop.DefaultNodes)
	if input.Limit != nil {
		limit = *input.Limit
	}
	start := 0
	if input.After == nil && len(entries) > int(limit) {
		start = len(entries) - int(limit)
	}
	end := min(len(entries), start+int(limit))
	page := entries[start:end]
	more := end < len(entries)
	next := history.Next()
	if more && len(page) > 0 {
		next = page[len(page)-1].Sequence + 1
	}
	return browserop.LogsResult{
		Entries:   page,
		Cursor:    history.Cursor(next),
		HasMore:   more,
		Truncated: history.HasEvicted(),
	}, nil
}

func (c *Console) collect(ctx context.Context, events *cdp.Subscription, history *cdp.Buffer[browserop.LogEntry]) {
	defer events.Close()
	for {
		raw, err := events.Next(ctx)
		if ctx.Err() != nil {
			return
		}
		var entry browserop.LogEntry
		if err == nil {
			entry, err = decodedConsole(raw)
		} else {
			var loss *cdp.EventLoss
			if !errors.As(err, &loss) {
				return
			}
		}
		c.mu.Lock()
		if err == nil {
			err = history.Push(entry)
		}
		if err != nil {
			err = history.MarkGap()
		}
		c.mu.Unlock()
		if err != nil {
			return
		}
	}
}

func consoleValue(argument *runtime.RemoteObject, text string) (string, error) {
	if argument.Value == nil {
		return text, nil
	}
	value, err := contract.JSON(argument.Value)
	if err != nil {
		return "", err
	}
	// CDP already validates the remote object's vendor envelope; parse the
	// retained JSON value through contract to preserve serde ordering/escaping.
	encoded, err := contract.EncodeJSON(value)
	if err != nil {
		return "", err
	}
	text = string(encoded)
	if len(encoded) > 0 && encoded[0] == '"' {
		var err error
		text, err = contract.Decode[string](encoded)
		if err != nil {
			return "", err
		}
	}

	return text, nil
}

func decodedConsole(raw cdp.Event) (browserop.LogEntry, error) {
	var entry browserop.LogEntry
	var err error
	var decoded any
	decoded, err = cdp.DecodeEvent(raw)
	if err == nil {
		if event, ok := decoded.(*runtime.EventConsoleAPICalled); ok {
			entry, err = consoleEntry(event)
		}
	}

	return entry, err
}
