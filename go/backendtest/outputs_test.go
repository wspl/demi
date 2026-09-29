package backendtest_test

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

const outputsConversation = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a05"

// Cost: one backend, a scripted vendor and a real device, several seconds: the
// device installs the builtin package, and six turns run a shell job each.
func TestALongOutputsResultNamesWhatItLeavesOutAndDemiShellOutputPrintsIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, outputsConversation)
	device := onDeviceConversation(t, b, master, outputsConversation)
	work := b.Open(master, vendor, outputsConversation, provider, "/work")

	// 30,000 lines, 168,894 bytes: beyond the 32 KiB of each stream the backend
	// receives while the command runs.
	counted := work.Turn(backendtest.ShellCall("t1", "seq 1 30000", 30*time.Second), backendtest.Say("counted"))
	result := counted.Result(t, 0)
	if n := len([]rune(result)); n > 16_000 {
		t.Fatalf("the result is %d characters", n)
	}
	command := backendtest.Field(t, result, "commandId")
	lines := strings.Split(strings.TrimSuffix(backendtest.ShownOutput(result), "\n"), "\n")
	if lines[0] != "1" || lines[len(lines)-1] != "30000" {
		t.Fatalf("the output runs from %q to %q", lines[0], lines[len(lines)-1])
	}
	between := -1
	for index, line := range lines {
		if strings.HasPrefix(line, "[... lines ") {
			between = index
			break
		}
	}
	if between < 0 {
		t.Fatal("no line between the start and the end")
	}
	before, _ := strconv.ParseUint(lines[between-1], 10, 64)
	after, _ := strconv.ParseUint(lines[between+1], 10, 64)
	first, last := before+1, after-1
	bytes := 0
	for number := first; number <= last; number++ {
		bytes += len(strconv.FormatUint(number, 10)) + 1
	}
	read := fmt.Sprintf("demi shell output %s --lines %d-%d", command, first, last)
	want := fmt.Sprintf("[... lines %d-%d not shown (%d bytes); read them: %s ...]", first, last, bytes, read)
	if lines[between] != want {
		t.Fatalf("the omission is %q, not %q", lines[between], want)
	}
	// The Host keeps nothing of the ended command.
	jobs := filepath.Join(device.Runner.StateDir(), "jobs")
	backendtest.Eventually(t, "the device keeps no job directory", func() bool {
		entries, _ := os.ReadDir(jobs)
		for _, entry := range entries {
			if entry.IsDir() {
				return false
			}
		}
		return true
	})

	// The command the line names prints those lines, a page at a time, numbered
	// as cat -n numbers them.
	paged := work.Turn(backendtest.ShellCall("t2", read, 30*time.Second), backendtest.Say("read"))
	page := strings.Split(strings.TrimSuffix(backendtest.ShownOutput(paged.Result(t, 0)), "\n"), "\n")
	if len(page) < 3 {
		t.Fatalf("the output page has fewer than three lines: %v", page)
	}
	if !strings.HasPrefix(page[0], fmt.Sprintf("[command %s: lines %d-", command, first)) || !strings.HasSuffix(page[0], " of 30000, stdout and stderr]") {
		t.Fatalf("the page begins %q", page[0])
	}
	if page[1] != fmt.Sprintf("%6d\t%d", first, first) {
		t.Fatalf("the first line is %q", page[1])
	}
	_, shownLine, found := strings.Cut(page[len(page)-2], "\t")
	if !found {
		t.Fatalf("the output line has no tab: %q", page[len(page)-2])
	}
	shown, _ := strconv.ParseUint(shownLine, 10, 64)
	next := fmt.Sprintf("[next: demi shell output %s --lines %d-%d]", command, shown+1, last)
	if page[len(page)-1] != next {
		t.Fatalf("the page ends %q, not %q", page[len(page)-1], next)
	}
	size := 0
	for _, line := range page {
		size += len([]rune(line)) + 1
	}
	if size > 12_000 {
		t.Fatalf("the page is %d characters", size)
	}

	// grep -n on the bytes gives the numbers pages take, and a reader that stops
	// early ends the call quietly.
	script := fmt.Sprintf("demi shell output %s --raw | grep -n '^12345$'; demi shell output %s --raw | head -n 2", command, command)
	searched := work.Turn(backendtest.ShellCall("t3", script, 30*time.Second), backendtest.Say("searched"))
	if got := backendtest.ShownOutput(searched.Result(t, 0)); got != "12345:12345\n1\n2\n" {
		t.Fatalf("the search shows %q", got)
	}

	// The newest lines; a range past the end, and a command the conversation does
	// not have, fail.
	script = fmt.Sprintf("demi shell output %s --tail 2; demi shell output %s --lines 30001-30002; demi shell output nothing-here", command, command)
	tailed := work.Turn(backendtest.ShellCall("t4", script, 30*time.Second), backendtest.Say("tailed"))
	wantTail := fmt.Sprintf("[command %s: lines 29999-30000 of 30000, stdout and stderr]\n 29999\t29999\n 30000\t30000\n", command) +
		"demi shell output: lines 30001-30002 are past the end: the output has 30000 lines\n" +
		"demi shell output: no command nothing-here in this conversation\n"
	if got := backendtest.ShownOutput(tailed.Result(t, 0)); got != wantTail {
		t.Fatalf("the tail shows %q, not %q", got, wantTail)
	}

	// A running command's output reads as its Host keeps it so far.
	started := work.Turn(backendtest.ShellCall("t5", "seq 1 20000; sleep 30", time.Second), backendtest.Say("running"))
	if !strings.HasPrefix(started.Result(t, 0), "status: running") {
		t.Fatalf("the command is %s", started.Result(t, 0))
	}
	running := backendtest.Field(t, started.Result(t, 0), "commandId")
	newest := work.Turn(backendtest.ShellCall("t6", "demi shell output "+running+" --tail 1", 30*time.Second), backendtest.Say("newest"))
	wantNewest := fmt.Sprintf("[command %s: lines 20000-20000 of 20000 so far, stdout and stderr]\n 20000\t20000\n", running)
	if got := backendtest.ShownOutput(newest.Result(t, 0)); got != wantNewest {
		t.Fatalf("the newest line shows %q, not %q", got, wantNewest)
	}
	b.Stop()
}
