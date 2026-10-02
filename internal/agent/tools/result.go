package tools

import (
	"context"
	"encoding/base64"
	"fmt"
	"slices"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

const (
	viewChars     = 32768
	videoCapBytes = 16 * 1024 * 1024
	runningNext   = "next: command is still running; check again with shell_status, or call yield to end this turn and be woken later, or shell_abort to stop it."
)

// shellOutcome renders the model's result and the independent page view.
func shellOutcome(ctx context.Context, status host.CommandStatus, model core.Model, limits provider.RequestLimits) session.ToolOutcome {
	var text host.OutputText
	if status.Whole != nil {
		var binaryLength *uint64
		if status.State.Phase == host.Exited && status.State.BinaryStdout != nil {
			binaryLength = &status.State.BinaryStdout.Info.TotalBytes
		}
		text = status.Whole.Output.Text(host.Both, binaryLength, status.Whole.Seen)
	} else {
		text = host.ReceivedOutput(status.Output.Text, status.Output.Line)
	}
	output := []provider.ResultPart{&provider.TextPart{Text: resultText(status, text)}}
	if status.State.Phase == host.Exited && status.State.BinaryStdout != nil {
		medium, note := binaryVerdict(ctx, status.State.BinaryStdout, status.CommandID, model, limits)
		if medium != nil {
			output = append(output, medium)
		}
		output = append(output, &provider.TextPart{Text: note})
	}
	return session.ToolOutcome{Output: output, View: &core.ShellView{ShellToolView: shellView(status, text)}}
}

// resultText budgets status, output and follow-up instructions together.
func resultText(status host.CommandStatus, text host.OutputText) string {
	command := status.CommandID
	running := status.State.Phase == host.Running
	before := []string{"status: " + string(viewStatus(status.State.Phase))}
	if status.State.Phase == host.Exited {
		before = append(before, fmt.Sprintf("exitCode: %d", status.State.ExitCode))
	}
	before = append(before, "commandId: "+string(command))
	if running {
		before = append(before, "shellId: "+string(status.ShellID), fmt.Sprintf("runningMs: %d", status.RunningMs), fmt.Sprintf("idleMs: %d", status.IdleMs))
	}
	var newest []host.Newest
	for _, item := range status.Newest {
		if running && item.Text != "" {
			newest = append(newest, item)
		}
	}
	var after []string
	if running && status.Unreceived > 0 && len(newest) == 0 {
		after = append(after, fmt.Sprintf("[... %d bytes not shown so far; the newest: demi shell output %s --tail 50 ...]", status.Unreceived, command))
	}
	switch status.State.Phase {
	case host.Running:
		hint := runningNext
		if status.State.Hint != nil {
			hint = *status.State.Hint
		}
		after = append(after, hint)
	case host.Aborted:
		after = append(after, "next: command was intentionally stopped.")
	}
	var markers []string
	for _, item := range newest {
		markers = append(markers, fmt.Sprintf("[... %d bytes of %s not shown; its newest lines follow ...]", item.LeftOut, item.Stream))
	}
	others := len("output:\n")
	for _, lines := range [][]string{before, after, markers} {
		for _, line := range lines {
			others += utf8.RuneCountInString(line) + 1
		}
	}
	budget := max(0, transcript.ReplayChars-others)
	startBudget := budget
	if len(newest) != 0 {
		startBudget /= 2
	}
	var shown []string
	if from, ok := text.UnseenLine(); ok {
		shown = cutOutput(text, from, command, startBudget)
	}
	used := 0
	for _, line := range shown {
		used += utf8.RuneCountInString(line) + 1
	}
	var newestLines []string
	if len(newest) != 0 {
		each := max(0, budget-used) / len(newest)
		for i, item := range newest {
			newestLines = append(newestLines, markers[i])
			tail := item.Text
			if _, rest, ok := strings.Cut(tail, "\n"); ok && item.LeftOut > 0 && rest != "" {
				tail = rest
			}
			lines, _ := takeEnd(host.ReceivedOutput(tail, 1), each)
			newestLines = append(newestLines, lines...)
		}
	}
	lines := before
	if len(shown) == 0 && len(newestLines) == 0 {
		lines = append(lines, "output: (empty)")
	} else {
		lines = append(lines, "output:")
		lines = append(lines, shown...)
		lines = append(lines, newestLines...)
	}
	lines = append(lines, after...)
	return strings.Join(lines, "\n")
}

// cutOutput retains all unseen output that fits, otherwise both ends and a read command.
func cutOutput(text host.OutputText, from uint64, command core.CommandID, budget int) []string {
	var all []string
	used := 0
	for piece := range text.Forward(from) {
		shown := piece.Text()
		used += utf8.RuneCountInString(shown) + 1
		if used > budget {
			length := len(text.Bytes())
			widest := max(utf8.RuneCountInString(linesMarker(command, text.LastLine(), text.LastLine(), length)), utf8.RuneCountInString(charsMarker(command, text.LastLine(), length, length))) + 1
			half := max(0, budget-widest) / 2
			head, start := takeStart(text, from, half)
			tail, end := takeEnd(text, half)
			head = append(head, outputMarker(text, command, start, max(start, end)))
			return append(head, tail...)
		}
		all = append(all, shown)
	}
	return all
}

// takeStart keeps whole output pieces or the beginning of the first long line.
func takeStart(text host.OutputText, from uint64, half int) ([]string, int) {
	var lines []string
	used, whole := 0, 0
	end := text.LineOffset(from)
	data := text.Bytes()
	for piece := range text.Forward(from) {
		shown := piece.Text()
		chars := utf8.RuneCountInString(shown)
		if used+chars < half {
			used += chars + 1
			if piece.Number != 0 {
				whole++
				end = piece.Offset + len(piece.Bytes)
				if end < len(data) && data[end] == '\n' {
					end++
				}
			}
			lines = append(lines, shown)
			continue
		}
		if whole == 0 && piece.Number != 0 {
			runes := []rune(shown)
			part := string(runes[:min(len(runes), max(0, half-1))])
			end = piece.Offset + min(len(part), len(piece.Bytes))
			lines = append(lines, part)
		}
		break
	}
	return lines, end
}

// takeEnd keeps whole output pieces or the ending of the last long line.
func takeEnd(text host.OutputText, half int) ([]string, int) {
	var lines []string
	used, whole := 0, 0
	start := len(text.Bytes())
	for piece := range text.Backward() {
		shown := piece.Text()
		chars := utf8.RuneCountInString(shown)
		if used+chars < half {
			used += chars + 1
			if piece.Number != 0 {
				whole++
				start = piece.Offset
			}
			lines = append(lines, shown)
			continue
		}
		if whole == 0 && piece.Number != 0 {
			part := string([]rune(shown)[max(0, chars-max(0, half-1)):])
			start = piece.Offset + len(piece.Bytes) - min(len(part), len(piece.Bytes))
			lines = append(lines, part)
		}
		break
	}
	slices.Reverse(lines)
	return lines, start
}

// outputMarker names omitted output by whole lines or columns within one line.
func outputMarker(text host.OutputText, command core.CommandID, start, end int) string {
	first := text.LineOf(start)
	last := text.LineOf(max(start, end-1))
	if first != last || text.IsLineStart(start) && text.IsLineStart(end) {
		return linesMarker(command, first, last, end-start)
	}
	through := text.Column(end)
	if end > start && text.IsLineStart(end) {
		through = text.Column(end - 1)
	}
	return charsMarker(command, first, text.Column(start)+1, through)
}

// linesMarker points to the shell command that reads omitted whole lines.
func linesMarker(command core.CommandID, first, last uint64, bytes int) string {
	return fmt.Sprintf("[... lines %d-%d not shown (%d bytes); read them: demi shell output %s --lines %d-%d ...]", first, last, bytes, command, first, last)
}

// charsMarker points to a page of omitted characters of one output line.
func charsMarker(command core.CommandID, line uint64, from, to int) string {
	return fmt.Sprintf("[... characters %d-%d of line %d not shown; read them: demi shell output %s --raw | sed -n %dp | cut -c %d-%d ...]", from, to, line, command, line, from, min(to, from+PageChars-1))
}

// binaryVerdict attaches accepted whole media, or explains how to save the bytes.
func binaryVerdict(ctx context.Context, binary *host.BinaryOutput, command core.CommandID, model core.Model, limits provider.RequestLimits) (provider.ResultPart, string) {
	total := binary.Info.TotalBytes
	save := fmt.Sprintf("save it: demi shell output %s --raw --stdout > <file>", command)
	if binary.Info.Truncated {
		return nil, fmt.Sprintf("Binary stdout (%d bytes) is more than the %d bytes a command's output keeps whole, so it was not attached and is kept whole nowhere; write it to a file instead and run the command again.", total, binary.Info.LimitBytes)
	}
	media := core.SniffModelMediaType(binary.Bytes)
	if media == nil {
		return nil, "Binary stdout does not match any model-viewable media type; " + save + "."
	}
	if !core.ModelAcceptsMediaType(model, media.MediaType) {
		return nil, fmt.Sprintf("Binary stdout is %s, which this model does not accept natively; %s.", media.MediaType, save)
	}
	if media.Kind == core.ModelMediaKindImage {
		fitted, err := store.Fit(ctx, binary.Bytes, media.MediaType)
		if err != nil {
			return nil, fmt.Sprintf("Binary stdout is %s (%d bytes), which was not attached because %s; %s.", media.MediaType, total, err, save)
		}
		note := fmt.Sprintf("Attached stdout as %s (%d bytes).", media.MediaType, total)
		if fitted.Reencoded {
			note = fmt.Sprintf("Attached stdout, %s of %dx%d px (%d bytes), as %s of %dx%d px (%d bytes), fitted to what every model accepts; to keep the original, %s.", media.MediaType, fitted.Came.Width, fitted.Came.Height, total, fitted.MediaType, fitted.Entered.Width, fitted.Entered.Height, len(fitted.Data), save)
		}
		return &provider.ResultImage{Bytes: provider.MediaBytes{Data: fitted.Data, MediaType: fitted.MediaType}}, note
	}
	if total > videoCapBytes {
		return nil, fmt.Sprintf("Binary stdout is %s (%d bytes), over the %d-byte video cap, so it was not attached; %s, or produce a smaller version, with fewer frames or a lower resolution, and run it again.", media.MediaType, total, videoCapBytes, save)
	}
	if limits.BodyBytes != nil {
		half := *limits.BodyBytes / 2
		if uint64(base64.StdEncoding.EncodedLen(len(binary.Bytes))) > half {
			return nil, fmt.Sprintf("Binary stdout is %s (%d bytes), whose base64 takes more than %d bytes, half of what this model's requests may carry, so it was not attached; %s, or produce a smaller version, with fewer frames or a lower resolution, and run it again.", media.MediaType, total, half, save)
		}
	}
	return &provider.ResultVideo{Bytes: provider.MediaBytes{Data: slices.Clone(binary.Bytes), MediaType: media.MediaType}}, fmt.Sprintf("Attached stdout as %s (%d bytes).", media.MediaType, total)
}

// viewStatus maps a shell environment phase to its transcript view.
func viewStatus(phase host.Phase) core.ShellViewStatus {
	switch phase {
	case host.Running:
		return core.ShellViewStatusRunning
	case host.Exited:
		return core.ShellViewStatusExited
	case host.Aborted:
		return core.ShellViewStatusAborted
	}
	return ""
}

// shellView records the end of unseen output, preserving stream tags and edited files.
func shellView(status host.CommandStatus, text host.OutputText) core.ShellToolView {
	var chunks []core.OutputChunk
	var cut bool
	if status.Whole != nil {
		if from, ok := text.Unseen(); ok {
			chunks = text.Chunks(from)
		}
		chunks, cut = tailWindow(chunks, viewChars)
	} else {
		chunks, cut = tailWindow(status.Output.Chunks, viewChars)
		cut = cut || status.Output.Truncated
	}
	view := core.ShellToolView{Status: viewStatus(status.State.Phase), ShellID: status.ShellID, CommandID: status.CommandID, RunningMs: status.RunningMs, IdleMs: status.IdleMs, Chunks: chunks, ViewTruncated: cut}
	if status.State.Phase == host.Exited {
		view.ExitCode = &status.State.ExitCode
	}
	if status.Files != nil {
		files := append([]core.EditedFile{}, status.Files.Files...)
		view.Files = &files
		truncated := status.Files.Truncated
		view.FilesTruncated = &truncated
	}
	return view
}

// tailWindow keeps the newest output characters without losing their stream tags.
func tailWindow(chunks []core.OutputChunk, limit int) ([]core.OutputChunk, bool) {
	kept := []core.OutputChunk{}
	total := 0
	for i := len(chunks) - 1; i >= 0; i-- {
		chunk := chunks[i]
		if chunk.Text == "" {
			continue
		}
		remaining := limit - total
		if remaining == 0 {
			slices.Reverse(kept)
			return kept, true
		}
		length := utf8.RuneCountInString(chunk.Text)
		if length <= remaining {
			kept = append(kept, chunk)
			total += length
			continue
		}
		chunk.Text = string([]rune(chunk.Text)[length-remaining:])
		kept = append(kept, chunk)
		slices.Reverse(kept)
		return kept, true
	}
	slices.Reverse(kept)
	return kept, false
}
