package tools

import (
	"fmt"
	"strconv"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/google/go-cmp/cmp"
	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// exited supplies a command whose whole stdout has not yet been seen.
func exited(text string) host.CommandStatus {
	output := host.WholeOutput{Records: []host.OutputRecord{{Stream: types.StreamKindStdout, Bytes: []byte(text)}}}
	return host.CommandStatus{
		ShellID:   "3",
		CommandID: "17",
		Output:    types.OutputView{Line: 1},
		Whole:     &host.WholeView{Output: &output},
		RunningMs: 5,
		IdleMs:    1,
		State:     host.CommandState{Phase: host.Exited, ExitCode: 1},
	}
}

// result reads the text a shell tool would return to the model.
func result(t *testing.T, status host.CommandStatus) string {
	t.Helper()
	outcome := shellOutcome(t.Context(), status, storetest.TestModel().Model, provider.RequestLimits{})
	return outcome.Output[0].(*provider.TextPart).Text
}

func TestOutputWithinBound(t *testing.T) {
	// Empty output must remain storable as a transcript view, not a nil array.
	empty := exited("")
	empty.Files = &host.EditedFiles{}
	outcome := shellOutcome(t.Context(), empty, storetest.TestModel().Model, provider.RequestLimits{})
	if _, err := (types.ToolViewJSON{Value: outcome.View}).MarshalJSON(); err != nil {
		t.Fatal(err)
	}
	got := result(t, exited("one\ntwo\n"))
	want := "status: exited\nexitCode: 1\ncommandId: 17\noutput:\none\ntwo"
	if got != want {
		t.Fatalf("result (-want +got):\n%s", cmp.Diff(want, got))
	}
}

func TestUnfinishedLineRepeatsUntilNewlineOrEnd(t *testing.T) {
	record := host.NewCommandRecord("3", "17", "call")
	record.AppendOutput(types.StreamKindStdout, "done\nre")
	record.AppendOutput(types.StreamKindStderr, "a")
	for _, want := range []string{"done\nrea", "rea"} {
		got := result(t, record.Status(0, nil))
		if !strings.HasSuffix(got, "\noutput:\n"+want+"\n"+runningNext) {
			t.Fatal(got)
		}
	}
	record.AppendOutput(types.StreamKindStdout, "dy\nprompt")
	for _, want := range []string{"ready\nprompt", "prompt"} {
		got := result(t, record.Status(0, nil))
		if !strings.HasSuffix(got, "\noutput:\n"+want+"\n"+runningNext) {
			t.Fatal(got)
		}
	}
	whole := &host.WholeOutput{
		Records: []host.OutputRecord{
			{Stream: types.StreamKindStdout, Bytes: []byte("done\nre")},
			{Stream: types.StreamKindStderr, Bytes: []byte("a")},
			{Stream: types.StreamKindStdout, Bytes: []byte("dy\nprompt")},
		},
	}
	record.Settle(host.Ending{Phase: host.Exited}, whole, nil, "")
	if got := result(t, record.Status(0, nil)); !strings.HasSuffix(got, "\noutput:\nprompt") {
		t.Fatal(got)
	}
	if got := result(t, record.Status(0, nil)); !strings.HasSuffix(got, "\noutput: (empty)") {
		t.Fatal(got)
	}
}

func TestLongOutputNamesOmittedLines(t *testing.T) {
	var numbers strings.Builder
	for i := 1; i <= 5000; i++ {
		fmt.Fprintf(&numbers, "%d\n", i)
	}
	text := result(t, exited(numbers.String()))
	if utf8.RuneCountInString(text) > transcript.ReplayChars {
		t.Fatal("result exceeds replay bound")
	}
	lines := strings.Split(text, "\n")
	marker := -1
	for i, line := range lines {
		if strings.HasPrefix(line, "[... lines ") {
			marker = i
			break
		}
	}
	if marker < 1 || marker+1 >= len(lines) {
		t.Fatal("missing omission marker")
	}
	before, err := strconv.Atoi(lines[marker-1])
	if err != nil {
		t.Fatal(err)
	}
	after, err := strconv.Atoi(lines[marker+1])
	if err != nil {
		t.Fatal(err)
	}
	if lines[4] != "1" || lines[len(lines)-1] != "5000" {
		t.Fatal("lost output endpoints")
	}
	bytes := 0
	for n := before + 1; n < after; n++ {
		bytes += len(fmt.Sprintf("%d\n", n))
	}
	want := fmt.Sprintf(
		"[... lines %d-%d not shown (%d bytes); read them: demi shell output 17 --lines %d-%d ...]",
		before+1,
		after-1,
		bytes,
		before+1,
		after-1,
	)
	if lines[marker] != want {
		t.Fatal(cmp.Diff(want, lines[marker]))
	}
	difference := before - (5000 - after)
	if difference <= -1000 || difference >= 1000 {
		t.Fatalf("unbalanced halves: %d %d", before, after)
	}
}

func TestSingleLongLineNamesOmittedCharacters(t *testing.T) {
	text := result(t, exited(strings.Repeat("x", 40000)))
	if utf8.RuneCountInString(text) > transcript.ReplayChars {
		t.Fatal("result exceeds replay bound")
	}
	lines := strings.Split(text, "\n")
	if len(lines) != 7 {
		t.Fatalf("got %d lines", len(lines))
	}
	from, to := len(lines[4])+1, 40000-len(lines[6])
	want := fmt.Sprintf(
		"[... characters %d-%d of line 1 not shown; read them: demi shell output 17 --raw | sed -n 1p | cut -c %d-%d ...]",
		from,
		to,
		from,
		from+PageChars-1,
	)
	if lines[5] != want {
		t.Fatal(cmp.Diff(want, lines[5]))
	}
}

func TestRunningStartAndNewestWithinBound(t *testing.T) {
	status := exited("")
	status.Whole = nil
	status.State = host.CommandState{Phase: host.Running}
	status.Output.Text = "building\n"
	status.Unreceived = 1048576
	status.Newest = []host.Newest{
		{Stream: types.StreamKindStdout, Offset: 1048576, LeftOut: 1040384, Text: "ne 998\nline 999\nline 1000\n"},
	}
	want := strings.Join(
		[]string{
			"status: running",
			"commandId: 17",
			"shellId: 3",
			"runningMs: 5",
			"idleMs: 1",
			"output:",
			"building",
			"[... 1040384 bytes of stdout not shown; its newest lines follow ...]",
			"line 999",
			"line 1000",
			runningNext,
		},
		"\n",
	)
	if got := result(t, status); got != want {
		t.Fatal(cmp.Diff(want, got))
	}
	status.Output.Text = strings.Repeat("a\n", 8000)
	status.Newest[0].Text = strings.Repeat("b\n", 8000)
	text := result(t, status)
	if utf8.RuneCountInString(text) > transcript.ReplayChars {
		t.Fatal("result exceeds replay bound")
	}
	first, newest := 0, 0
	for line := range strings.SplitSeq(text, "\n") {
		if line == "a" {
			first++
		}
		if line == "b" {
			newest++
		}
	}
	if first <= 3000 || newest <= 3000 {
		t.Fatalf("too little output: %d %d", first, newest)
	}
	if !strings.Contains(text, "bytes of stdout not shown; its newest lines follow") {
		t.Fatal("missing newest marker")
	}
}

func TestRunningHandlesAndUnreceivedOutput(t *testing.T) {
	status := exited("")
	status.Whole = nil
	status.State = host.CommandState{Phase: host.Running}
	status.Output.Text = "building\n"
	status.Unreceived = 1048576
	want := strings.Join(
		[]string{
			"status: running",
			"commandId: 17",
			"shellId: 3",
			"runningMs: 5",
			"idleMs: 1",
			"output:",
			"building",
			"[... 1048576 bytes not shown so far; the newest: demi shell output 17 --tail 50 ...]",
			runningNext,
		},
		"\n",
	)
	if got := result(t, status); got != want {
		t.Fatal(cmp.Diff(want, got))
	}
	hint := "waiting for input: answer with shell_write"
	status.State.Hint = &hint
	if got := result(t, status); !strings.HasSuffix(got, "\n"+hint) {
		t.Fatal(got)
	}
	aborted := exited("")
	aborted.State = host.CommandState{Phase: host.Aborted}
	want = "status: aborted\ncommandId: 17\noutput: (empty)\nnext: command was intentionally stopped."
	if got := result(t, aborted); got != want {
		t.Fatal(cmp.Diff(want, got))
	}
}

func TestBinaryStdoutAttachmentPolicy(t *testing.T) {
	png := storetest.PNG(4, 3, 1)
	wide := storetest.PNG(2400, 10, 1)
	model := storetest.ModelReading("stub", "test", []types.FileExtension{types.FileExtensionPNG}).Model
	videoModel := storetest.ModelReading("stub", "test", []types.FileExtension{types.FileExtensionMP4}).Model
	mp4 := []byte("\x00\x00\x00\x20ftypisom\xff\xfe")
	save := "save it: demi shell output 17 --raw --stdout > <file>"
	body40, body39 := uint64(40), uint64(39)
	cases := []struct {
		name                 string
		data                 []byte
		model                types.Model
		body                 *uint64
		total                uint64
		truncated            bool
		want, prefix, suffix string
	}{
		{
			name:  "unchanged image",
			data:  png,
			model: model,
			want:  fmt.Sprintf("<image> | Attached stdout as image/png (%d bytes).", len(png)),
		},
		{
			name:  "fitted image",
			data:  wide,
			model: model,
			prefix: fmt.Sprintf(
				"<image> | Attached stdout, image/png of 2400x10 px (%d bytes), as image/png of 2000x8 px (",
				len(wide),
			),
			suffix: " bytes), fitted to what every model accepts; to keep the original, " + save + ".",
		},
		{
			name:   "broken image",
			data:   []byte("\x89PNG\r\n\x1a\n\x00\xff\xfe\x01"),
			model:  model,
			prefix: "Binary stdout is image/png (12 bytes), which was not attached because it could not be decoded",
			suffix: "; " + save + ".",
		},
		{
			name:  "unsupported video",
			data:  mp4,
			model: model,
			want:  "Binary stdout is video/mp4, which this model does not accept natively; " + save + ".",
		},
		{
			name:  "video at body bound",
			data:  mp4,
			model: videoModel,
			body:  &body40,
			want:  "<video> | Attached stdout as video/mp4 (14 bytes).",
		},
		{
			name:  "video exceeds body",
			data:  mp4,
			model: videoModel,
			body:  &body39,
			want: "Binary stdout is video/mp4 (14 bytes), whose base64 takes more than 19 bytes, half " +
				"of what this model's requests may carry, so it was not attached; " +
				save +
				", or produce a smaller version, with fewer frames or a lower resolution, and run it " +
				"again.",
		},
		{
			name:  "video cap",
			data:  mp4,
			model: videoModel,
			total: 17 * 1024 * 1024,
			want: "Binary stdout is video/mp4 (17825792 bytes), over the 16777216-byte video cap, so it " +
				"was not attached; " +
				save +
				", or produce a smaller version, with fewer frames or a lower resolution, and run it " +
				"again.",
		},
		{
			name:  "opaque",
			data:  []byte("\xde\xad\xbe\xef\xff\xfe\x00\x01\x02\x03\x04\x05"),
			model: model,
			want:  "Binary stdout does not match any model-viewable media type; " + save + ".",
		},
		{
			name:      "truncated",
			data:      png,
			model:     model,
			total:     20000000,
			truncated: true,
			want: "Binary stdout (20000000 bytes) is more than the 16777216 bytes a command's output " +
				"keeps whole, so it was not attached and is kept whole nowhere; write it to a file " +
				"instead and run the command again.",
		},
	}
	for _, scenario := range cases {
		t.Run(scenario.name, func(t *testing.T) {
			total := scenario.total
			if total == 0 {
				total = uint64(len(scenario.data))
			}
			status := exited("")
			status.State.BinaryStdout = &host.BinaryOutput{
				Bytes: scenario.data,
				Info: types.BinaryStdout{
					TotalBytes: total,
					LimitBytes: 16 * 1024 * 1024,
					Truncated:  scenario.truncated,
				},
			}
			outcome := shellOutcome(
				t.Context(),
				status,
				scenario.model,
				provider.RequestLimits{BodyBytes: scenario.body},
			)
			var parts []string
			for _, part := range outcome.Output[1:] {
				switch p := part.(type) {
				case *provider.TextPart:
					parts = append(parts, p.Text)
				case *provider.ResultImage:
					parts = append(parts, "<image>")
				case *provider.ResultVideo:
					parts = append(parts, "<video>")
				}
			}
			got := strings.Join(parts, " | ")
			if scenario.want != "" && got != scenario.want {
				t.Fatal(cmp.Diff(scenario.want, got))
			}
			if scenario.prefix != "" &&
				(!strings.HasPrefix(got, scenario.prefix) || !strings.HasSuffix(got, scenario.suffix)) {
				t.Fatal(got)
			}
		})
	}
}

func TestViewWindowKeepsNewestMergedCharacters(t *testing.T) {
	chunks := []types.OutputChunk{
		{Stream: types.StreamKindStdout, Text: "ab"},
		{Stream: types.StreamKindStderr},
		{Stream: types.StreamKindStderr, Text: "cdé"},
	}
	for _, scenario := range []struct {
		limit int
		want  []types.OutputChunk
		cut   bool
	}{
		{10, []types.OutputChunk{chunks[0], chunks[2]}, false},
		{4, []types.OutputChunk{{Stream: types.StreamKindStdout, Text: "b"}, chunks[2]}, true},
		{3, []types.OutputChunk{chunks[2]}, true},
	} {
		t.Run(strconv.Itoa(scenario.limit), func(t *testing.T) {
			got, cut := tailWindow(chunks, scenario.limit)
			if diff := cmp.Diff(scenario.want, got); diff != "" || cut != scenario.cut {
				t.Fatalf("chunks %s; cut %v want %v", diff, cut, scenario.cut)
			}
		})
	}
}
