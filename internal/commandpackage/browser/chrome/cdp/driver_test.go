package cdp_test

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func mustValue(t *testing.T, value any) json.RawMessage {
	t.Helper()
	raw, err := cdp.Value(value)
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func mustInput(t *testing.T, name string, args string) browserproto.Input {
	t.Helper()
	input, err := browserproto.ParseInput(name, []byte(args))
	if err != nil {
		t.Fatal(err)
	}
	return input
}

func requireCode(t *testing.T, err error, code string) {
	t.Helper()
	if string(cdp.ErrorCode(err)) != code {
		t.Fatalf("error %v has code %s, want %s", err, cdp.ErrorCode(err), code)
	}
}

func TestDeadlineCauseSurvivesCleanupWrappers(t *testing.T) {
	deadline := &cdp.BrowserError{Kind: cdp.KindTimeout}
	err := cdp.AfterCleanup(deadline, deadline)
	if !cdp.IsDeadline(err) {
		t.Fatal(err)
	}
	condition, interceptor := "hit", "overlay"
	cause := &cdp.BrowserError{
		Kind:    cdp.KindNotActionable,
		Details: browserproto.ErrorDetails{Condition: &condition, Interceptor: &interceptor},
	}
	err = cdp.WithDeadlineCause(err, cause)
	requireCode(t, err, "not_actionable")
	details := cdp.ErrorDetails(err)
	if details.Condition == nil || *details.Condition != condition || details.Interceptor == nil ||
		*details.Interceptor != interceptor {
		t.Fatal(details)
	}
	var wrapper *cdp.BrowserError
	if !errors.As(err, &wrapper) || wrapper.Cleanup != deadline {
		t.Fatal(err)
	}
}

func TestControllerDeadlineCancellationIsATimeout(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		expired := cdp.OperationUntil(ctx, t.Context(), time.Now())
		defer expired.Close()
		requireCode(t, expired.Run(ctx, func(context.Context) error {
			t.Fatal("expired step ran")
			return nil
		}), "timeout")
		active := cdp.NewOperation(ctx, t.Context(), cdp.ControlTimeout)
		defer active.Close()
		requireCode(t, active.Run(ctx, func(context.Context) error {
			t.Fatal("cancelled step ran")
			return nil
		}), "cancelled")
		timed := cdp.NewOperation(t.Context(), t.Context(), time.Second)
		defer timed.Close()
		start := time.Now()
		requireCode(t, timed.Run(t.Context(), func(ctx context.Context) error {
			<-ctx.Done()
			return ctx.Err()
		}), "timeout")
		if elapsed := time.Since(start); elapsed != time.Second {
			t.Fatal(elapsed)
		}
	})
}

func TestTransportEndRetainsItsCauseWhileExplicitCloseStaysClosed(t *testing.T) {
	for _, lost := range []bool{false, true} {
		lifetime, end := context.WithCancelCause(t.Context())
		operation := cdp.NewOperation(t.Context(), lifetime, cdp.ControlTimeout)
		operation.BeginInput()
		if lost {
			end(&cdp.BrowserError{Kind: cdp.KindConnection, Message: "transport ended"})
		} else {
			end(nil)
		}
		err := operation.Run(t.Context(), func(context.Context) error {
			t.Fatal("closed operation ran")
			return nil
		})
		err = operation.Failure(err, "tab", nil)
		if lost {
			requireCode(t, err, "outcome_unknown")
			if action := cdp.ErrorDetails(err).Action; action == nil || *action != "unknown" {
				t.Fatalf("action = %v", action)
			}
			if !strings.Contains(err.Error(), "transport ended") {
				t.Fatal(err)
			}
		} else {
			requireCode(t, err, "browser_lost")
		}
		operation.Close()
	}
}

func TestConnectionLossAfterDispatchHasAnUnknownOutcome(t *testing.T) {
	operation := cdp.NewOperation(t.Context(), t.Context(), cdp.ControlTimeout)
	defer operation.Close()
	lost := &cdp.BrowserError{Kind: cdp.KindConnection, Message: "transport disconnected"}
	url := "https://example.test"
	before := operation.Failure(lost, "tab", &url)
	requireCode(t, before, "browser_lost")
	if *cdp.ErrorDetails(before).Action != "not_started" {
		t.Fatal(before)
	}
	operation.BeginInput()
	for _, completed := range []bool{false, true} {
		if completed {
			operation.CompleteInput()
		}
		err := operation.Failure(cdp.AfterCleanup(lost, &cdp.BrowserError{Kind: cdp.KindClosed}), "tab", &url)
		requireCode(t, err, "outcome_unknown")
		details := cdp.ErrorDetails(err)
		want := browserproto.ErrorDetails{Action: new(browserproto.ActionProgressUnknown), Tab: new("tab"), URL: &url}
		if !reflect.DeepEqual(details, want) || !strings.Contains(err.Error(), lost.Message) {
			t.Fatal(err, details)
		}
		requireCode(t, operation.Failure(&cdp.BrowserError{Kind: cdp.KindClosed}, "tab", nil), "browser_lost")
		requireCode(t, operation.Failure(&cdp.BrowserError{Kind: cdp.KindCancelled}, "tab", nil), "cancelled")
	}
}

func TestBrowserErrorsKeepCausesAndInputProgress(t *testing.T) {
	for kind, code := range map[cdp.ErrorKind]string{
		cdp.KindTabNotFound:      "tab_not_found",
		cdp.KindClosed:           "browser_lost",
		cdp.KindTargetNotFound:   "target_not_found",
		cdp.KindAmbiguous:        "ambiguous_target",
		cdp.KindHistoryBoundary:  "history_boundary",
		cdp.KindNavigationFailed: "navigation_failed",
		cdp.KindOutputExists:     "output_exists",
		cdp.KindIO:               "io_error",
		cdp.KindConfiguration:    "invalid_input",
	} {
		requireCode(t, &cdp.BrowserError{Kind: kind}, code)
	}
	operation := cdp.NewOperation(t.Context(), t.Context(), cdp.ControlTimeout)
	defer operation.Close()
	for _, progress := range []browserproto.ActionProgress{"not_started", "unknown", "completed"} {
		if progress == "unknown" {
			operation.BeginInput()
		}
		if progress == "completed" {
			operation.CompleteInput()
		}
		kind := cdp.KindTimeout
		if progress == "unknown" {
			kind = cdp.KindClosed
		}
		err := operation.Failure(&cdp.BrowserError{Kind: kind}, "tab", nil)
		if *cdp.ErrorDetails(err).Action != progress {
			t.Fatal(err)
		}
	}
	primary := &cdp.BrowserError{Kind: cdp.KindIO, Cause: os.ErrPermission}
	if !errors.Is(cdp.AfterCleanup(primary, context.Canceled), os.ErrPermission) {
		t.Fatal("IO cause lost")
	}
}

func TestBrowserStreamsKeepIndependentCursorsAndReportCountAndByteEviction(t *testing.T) {
	create := func() *cdp.Buffer[browserproto.LogEntry] {
		b, err := cdp.NewBuffer("test", 2, 160, func(e browserproto.LogEntry) uint64 {
			return e.Sequence
		}, func(e browserproto.LogEntry, n uint64) browserproto.LogEntry {
			e.Sequence = n
			return e
		})
		if err != nil {
			t.Fatal(err)
		}
		return b
	}
	buffer := create()
	start := buffer.Cursor(0)
	for _, text := range []string{"a", "b", "c"} {
		if err := buffer.Push(browserproto.LogEntry{Level: "log", Text: text}); err != nil {
			t.Fatal(err)
		}
	}
	entries := buffer.Entries()
	if len(entries) != 2 || entries[0].Sequence != 1 || entries[1].Sequence != 2 {
		t.Fatal(entries)
	}
	if position, err := buffer.Position(start); err != nil || position != 0 {
		t.Fatal(position, err)
	}
	if !buffer.TruncatedSince(0) || buffer.TruncatedSince(1) {
		t.Fatal("wrong eviction marker")
	}
	if _, err := create().Position(start); err == nil {
		t.Fatal("foreign cursor accepted")
	}
	if _, err := buffer.Position(buffer.Cursor(4)); err == nil {
		t.Fatal("future cursor accepted")
	}
	if err := buffer.Push(browserproto.LogEntry{Level: "log", Text: strings.Repeat("x", 160)}); err != nil {
		t.Fatal(err)
	}
	if len(buffer.Entries()) != 0 || !buffer.TruncatedSince(3) {
		t.Fatal("oversized entry retained")
	}
	if err := buffer.MarkGap(); err != nil {
		t.Fatal(err)
	}
	if buffer.Next() != 5 || !buffer.TruncatedSince(4) || buffer.TruncatedSince(5) {
		t.Fatal("gap not observable")
	}
}

func TestStreamOutputTruncationDoesNotSkipTheOmittedEntries(t *testing.T) {
	entries := []browserproto.LogEntry{}
	for sequence := uint64(0); sequence < 3; sequence++ {
		entries = append(
			entries,
			browserproto.LogEntry{Sequence: sequence, Level: "info", Text: strings.Repeat("x", 30000)},
		)
	}
	value := mustValue(t, browserproto.LogsResult{Entries: entries, Cursor: "logs_test:3"})
	output, err := cdp.Render(mustInput(t, "logs", `{"tab":"t1"}`), value, true)
	if err != nil {
		t.Fatal(err)
	}
	result, err := browserproto.DecodeLogsResult(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Entries) != 2 || result.Cursor != "logs_test:2" || !result.HasMore || !result.Truncated ||
		len(output) > browserproto.InlineBytes {
		t.Fatal(result.Cursor, len(result.Entries))
	}
}

func TestTextErrorsShowProgressAndIndividualDetails(t *testing.T) {
	action := browserproto.ActionProgress("not_started")
	tab, url, interceptor := "t1", "https://example.test/", "<div id=overlay>"
	delivered := uint(0)
	text := cdp.RenderError(
		"not_actionable",
		"The button is covered.",
		browserproto.ErrorDetails{
			Action:      &action,
			Tab:         &tab,
			URL:         &url,
			Interceptor: &interceptor,
			Delivered:   &delivered,
		},
	)
	if !strings.HasPrefix(text, "Error: not_actionable\nThe button is covered.\nAction: not_started.\n") {
		t.Fatal(text)
	}
	for _, line := range []string{
		"Tab: t1\n",
		"Current URL: https://example.test/\n",
		"Interceptor: <div id=overlay>\n",
		"Delivered: 0\n",
	} {
		if !strings.Contains(text, line) {
			t.Fatal(text, line)
		}
	}
	if strings.Contains(text, "Details:") {
		t.Fatal(text)
	}
}

func TestEveryResultRendersAsReadableLines(t *testing.T) {
	bytes, err := os.ReadFile("testdata/text_cases.json")
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct {
		Operation string
		Args      json.RawMessage
		Result    json.RawMessage
		Lines     []string
	}
	if err := json.Unmarshal(bytes, &cases); err != nil {
		t.Fatal(err)
	}
	spaces := regexp.MustCompile(` +`)
	for _, test := range cases {
		t.Run(test.Operation, func(t *testing.T) {
			operation, err := browserproto.ParseInput(test.Operation, test.Args)
			if err != nil {
				t.Fatal(err)
			}
			output, err := cdp.Render(operation, test.Result, false)
			if err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(string(output), "{") {
				t.Fatalf("%s printed JSON: %s", test.Operation, output)
			}
			text := spaces.ReplaceAllString(string(output), " ")
			for _, line := range test.Lines {
				if !strings.Contains(text, spaces.ReplaceAllString(line, " ")) {
					t.Fatalf("missing %q in\n%s", line, output)
				}
			}
		})
	}
}

func TestPageTextStaysInsideItsValue(t *testing.T) {
	operation := mustInput(t, "open", `{"url":"http://localhost:3000/"}`)
	title := "Sign in\nError: forged\r\x1b[31m"
	text, err := cdp.Render(
		operation,
		mustValue(t, browserproto.OpenResult{Tab: "t1", URL: "http://localhost:3000/", Title: &title}),
		false,
	)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(text), `Title: Sign in\nError: forged\r\x1b[31m`+"\n") ||
		strings.Contains(string(text), "\x1b") ||
		strings.Contains(string(text), "\nError:") {
		t.Fatal(string(text))
	}
	content := json.RawMessage(
		`{"url":"http://localhost:3000/","title":"Page","format":"text","content":"First ` +
			`line\nSecond\u001b[2J line","truncated":false}`,
	)
	text, err = cdp.Render(mustInput(t, "content.read", `{"tab":"t1"}`), content, false)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(text), "First line\nSecond\\x1b[2J line\n") {
		t.Fatal(string(text))
	}
}

func TestBrowserPublicationsAndConversationNumbers(t *testing.T) {
	directory := t.TempDir()
	path, err := cdp.SaveWithOverwrite(t.Context(), directory, "output", []byte("first"), false)
	if err != nil {
		t.Fatal(err)
	}
	_, err = cdp.SaveWithOverwrite(t.Context(), directory, "output", []byte("second"), false)
	requireCode(t, err, "output_exists")
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if _, err := cdp.SaveWithOverwrite(ctx, directory, "output", []byte("late"), true); err == nil {
		t.Fatal("cancelled output published")
	}
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "first" {
		t.Fatal(string(data), err)
	}
	source := filepath.Join(directory, "download")
	if err := os.WriteFile(source, []byte("downloaded"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := cdp.PublishFile(t.Context(), directory, "output", source, true); err != nil {
		t.Fatal(err)
	}
	data, err = os.ReadFile(path)
	if err != nil || string(data) != "downloaded" {
		t.Fatal(string(data), err)
	}
	numbers := cdp.NewTabNumbers(commandsdktest.CountingNumbers(t), "conversation")
	seen := map[uint64]bool{}
	for range 17 {
		number, err := numbers.Next(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		if seen[number] {
			t.Fatal("reused number", number)
		}
		seen[number] = true
	}
}

// Releasing an operation twice must finish even when its lifetime is still active.
func TestOperationCloseReleasesOnce(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		operation := cdp.NewOperation(t.Context(), t.Context(), cdp.ControlTimeout)
		operation.Close()
		operation.Close()
		if operation.Context().Err() == nil {
			t.Fatal("operation context remains active")
		}
	})
}
