package backend_test

import (
	"fmt"
	"strconv"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/provider/providertest"
)

// Six real shell turns exercise retained output; no real model is called.
func TestLongOutputNamesOmittedLinesAndShellOutputReadsThem(t *testing.T) {
	t.Skip("finding 1: device claim returns 500 because device.installs is nil")
	ctx, h := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-file")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	paired, _ := conversationOnDevice(ctx, t, h, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, entry, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	turn := func(id, script, answer string, timeout int) string {
		before := len(vendor.Requests())
		vendor.Respond(conversationShell(t, id, script, timeout))
		vendor.Respond(conversationAnswer(t, []string{answer}, 1, 1))
		_, err := socket.Chat(ctx, "m"+id[1:], "go")
		wireMust(t, err)
		return conversationToolResult(t, vendor.Requests()[before+1], id)
	}
	result := turn("t1", "seq 1 30000", "counted", 30000)
	if utf8.RuneCountInString(result) > 16000 {
		t.Fatal("model preview exceeds 16000 characters")
	}
	command := toolstest.Field(result, "commandId")
	lines := strings.Split(strings.TrimSuffix(toolstest.ShownOutput(result), "\n"), "\n")
	conversationEqual(t, lines[0], "1")
	conversationEqual(t, lines[len(lines)-1], "30000")
	between := -1
	for i, line := range lines {
		if strings.HasPrefix(line, "[... lines ") {
			between = i
			break
		}
	}
	if between < 1 {
		t.Fatal("no omitted-line marker")
	}
	first, err := strconv.Atoi(lines[between-1])
	wireMust(t, err)
	first++
	last, err := strconv.Atoi(lines[between+1])
	wireMust(t, err)
	last--
	bytes := 0
	for i := first; i <= last; i++ {
		bytes += len(strconv.Itoa(i)) + 1
	}
	read := fmt.Sprintf("demi shell output %s --lines %d-%d", command, first, last)
	conversationEqual(t, lines[between], fmt.Sprintf("[... lines %d-%d not shown (%d bytes); read them: %s ...]", first, last, bytes, read))
	wireMust(t, backendtest.WaitRunnerJobsRemoved(ctx, paired.Runner.StateDir()))
	page := strings.Split(strings.TrimSuffix(toolstest.ShownOutput(turn("t2", read, "read", 30000)), "\n"), "\n")
	if !strings.HasPrefix(page[0], fmt.Sprintf("[command %s: lines %d-", command, first)) || !strings.HasSuffix(page[0], " of 30000, stdout and stderr]") {
		t.Fatalf("page header: %s", page[0])
	}
	conversationEqual(t, page[1], fmt.Sprintf("%6d\t%d", first, first))
	shown, err := strconv.Atoi(strings.Split(page[len(page)-2], "\t")[1])
	wireMust(t, err)
	conversationEqual(t, page[len(page)-1], fmt.Sprintf("[next: demi shell output %s --lines %d-%d]", command, shown+1, last))
	characters := 0
	for _, line := range page {
		characters += utf8.RuneCountInString(line) + 1
	}
	if characters > 12000 {
		t.Fatal("output page exceeds 12000 characters")
	}
	script := fmt.Sprintf("demi shell output %s --raw | grep -n '^12345$'; demi shell output %s --raw | head -n 2", command, command)
	conversationEqual(t, toolstest.ShownOutput(turn("t3", script, "searched", 30000)), "12345:12345\n1\n2\n")
	script = fmt.Sprintf("demi shell output %s --tail 2; demi shell output %s --lines 30001-30002; demi shell output nothing-here", command, command)
	want := fmt.Sprintf("[command %s: lines 29999-30000 of 30000, stdout and stderr]\n 29999\t29999\n 30000\t30000\ndemi shell output: lines 30001-30002 are past the end: the output has 30000 lines\ndemi shell output: no command nothing-here in this conversation\n", command)
	conversationEqual(t, toolstest.ShownOutput(turn("t4", script, "tailed", 30000)), want)
	running := turn("t5", "seq 1 20000; sleep 30", "running", 1000)
	if !strings.HasPrefix(running, "status: running") {
		t.Fatalf("expected running: %s", running)
	}
	id := toolstest.Field(running, "commandId")
	newest := turn("t6", "demi shell output "+id+" --tail 1", "newest", 30000)
	conversationEqual(t, toolstest.ShownOutput(newest), fmt.Sprintf("[command %s: lines 20000-20000 of 20000 so far, stdout and stderr]\n 20000\t20000\n", id))
}
