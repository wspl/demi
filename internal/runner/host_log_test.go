package runner

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runnerproto"
)

// These stored-log scenarios run through the program's log_read wire boundary.
// The rotation scenario writes 4 MiB and normally completes in about one second.
func seededLine(t *testing.T, seq uint64, source string, conversation *string, text string) string {
	t.Helper()
	bytes, err := (logLine{
		Seq: seq, At: 1700000000000, Source: source, ConversationID: conversation, Text: text,
	}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	return string(bytes) + "\n"
}

func (f *runnerFixture) seedLog(older, newer string) {
	f.stop()
	for name, text := range map[string]string{"host.log.1": older, "host.log": newer} {
		if err := os.WriteFile(filepath.Join(f.state, "log", name), []byte(text), 0o600); err != nil {
			f.t.Fatal(err)
		}
	}
	f.start()
	f.online()
}

func (f *runnerFixture) readLog(since *uint64, limit uint64, source *string) *runnerproto.LogLines {
	f.t.Helper()
	f.send(&runnerproto.LogRead{ID: "read", Since: since, Limit: limit, Source: source})
	lines, ok := f.frame().(*runnerproto.LogLines)
	if !ok || lines.ID != "read" {
		f.t.Fatal("log read failed or returned the wrong request ID")
	}
	return lines
}

func logTexts(lines []runnerproto.LogLine) string {
	var text []string
	for _, line := range lines {
		text = append(text, line.Text)
	}
	return strings.Join(text, "|")
}

func TestLogPageHoldsCursorLimitAndSource(t *testing.T) {
	f := newRunner(t, nil, "")
	older := seededLine(
		t,
		100,
		"service:demi.browser",
		nil,
		"tabs failed",
	) + seededLine(
		t,
		101,
		"stream:browser.live",
		new("conversation"),
		"no tabs",
	)
	newer := seededLine(t, 102, "service:demi.browser", nil, "tabs listed") + `{"seq":`
	f.seedLog(older, newer)
	page := f.readLog(nil, 10, new("service:demi.browser"))
	if logTexts(page.Lines) != "tabs failed|tabs listed" {
		t.Fatal(page)
	}
	page = f.readLog(nil, 1, new("service:demi.browser"))
	if logTexts(page.Lines) != "tabs listed" {
		t.Fatal(page)
	}
	page = f.readLog(new(uint64(0)), 2, nil)
	if logTexts(page.Lines) != "tabs failed|no tabs" || page.Next != 101 || page.Lines[1].ConversationID == nil ||
		*page.Lines[1].ConversationID != "conversation" {
		t.Fatal(page)
	}
	page = f.readLog(new(uint64(101)), 1, nil)
	if logTexts(page.Lines) != "tabs listed" || page.Next != 102 {
		t.Fatal(page)
	}
	page = f.readLog(new(uint64(102)), 1, nil)
	if len(page.Lines) != 1 || page.Lines[0].Source != "runner" || page.Next != 103 {
		t.Fatal(page)
	}
	page = f.readLog(new(^uint64(0)), 1, nil)
	if logTexts(page.Lines) != "tabs failed" || page.Next != 100 {
		t.Fatal(page)
	}
}

func TestLongLogLinesRotateAndCursorReadsEachOnce(t *testing.T) {
	f := newRunner(t, nil, "")
	text := strings.Repeat("x", 4000)
	var newer strings.Builder
	seq := uint64(1000)
	for newer.Len()+2*4100 < logFileBytes {
		newer.WriteString(seededLine(t, seq, "service:demi.fill", nil, text))
		seq++
	}
	count := seq - 1000
	padding := logFileBytes - newer.Len() - 150
	empty := len(seededLine(t, seq, "service:demi.pad", nil, ""))
	newer.WriteString(seededLine(t, seq, "service:demi.pad", nil, strings.Repeat("x", padding-empty)))
	f.seedLog("", newer.String())
	// log_read is a FIFO barrier after the startup log writes, not a timed wait.
	f.readLog(nil, 1000, new("runner"))
	older, err := os.Stat(filepath.Join(f.state, "log", "host.log.1"))
	if err != nil || older.Size() > logFileBytes || older.Size() < logFileBytes/2 {
		t.Fatalf("rotation: %v, %v", older, err)
	}
	var found uint64
	since := uint64(0)
	for {
		page := f.readLog(&since, 1000, new("service:demi.fill"))
		if len(page.Lines) == 0 {
			break
		}
		bytes := 0
		for _, line := range page.Lines {
			if line.Text != text {
				t.Fatal("log line changed")
			}
			bytes += len(line.Text)
		}
		if bytes > logPageBytes {
			t.Fatal("page exceeds byte bound")
		}
		found += uint64(len(page.Lines))
		since = page.Next
	}
	if found != count {
		t.Fatalf("read %d lines, want %d", found, count)
	}
}
