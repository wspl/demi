package remotehost

import (
	"context"
	"errors"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/text/encoding/unicode"
	"golang.org/x/text/transform"
)

// receivedStream holds the first output view and the bounded newest tail of one stream.
type receivedStream struct {
	head, length, next uint64
	pending            []byte
	newest             []byte
	newestOffset       uint64
}

// decodeOutputText adapts the UTF-8 library to an incrementally received command stream.
func decodeOutputText(pending *[]byte, data []byte, final bool) string {
	source := append(*pending, data...)
	output := make([]byte, 3*len(source)+3)
	n, consumed, err := unicode.UTF8.NewDecoder().Transform(output, source, final)
	// The decoder replaces malformed bytes. With triple-sized output its only
	// possible error is ErrShortSrc, which retains the unfinished character.
	if err != nil && !errors.Is(err, transform.ErrShortSrc) {
		slog.Error("command output decode failed: " + err.Error())
	}
	*pending = append([]byte(nil), source[consumed:]...)
	return string(output[:n])
}

// receive updates command views with only new bytes while retaining a stream's newest tail.
func (s *receivedStream) receive(record *host.CommandRecord, chunk JobOutput, received *[]host.OutputRecord) bool {
	end := chunk.Offset + uint64(len(chunk.Bytes))
	s.length = max(s.length, end)
	if chunk.Offset < uint64(runnerwire.JobViewBytes) {
		s.head = end
		s.next = end
		text := decodeOutputText(&s.pending, chunk.Bytes, false)
		if len(chunk.Bytes) > 0 {
			*received = append(*received, host.OutputRecord{Stream: chunk.Stream, Bytes: chunk.Bytes})
		}
		return record.AppendOutput(chunk.Stream, text)
	}
	record.Grew(chunk.Stream, end)
	if len(chunk.Bytes) == 0 {
		return false
	}
	previousEnd := s.newestOffset + uint64(len(s.newest))
	if len(s.newest) == 0 || chunk.Offset < s.newestOffset || chunk.Offset > previousEnd {
		s.newest = append([]byte(nil), chunk.Bytes...)
		s.newestOffset = chunk.Offset
	} else {
		s.newest = append(s.newest[:int(chunk.Offset-s.newestOffset)], chunk.Bytes...)
	}
	excess := max(0, len(s.newest)-runnerwire.JobViewBytes)
	s.newest = s.newest[excess:]
	s.newestOffset += uint64(excess)
	tail := s.newest
	for n := 0; n < 3 && len(tail) > 0 && tail[0]&0xc0 == 0x80; n++ {
		tail = tail[1:]
	}
	var pending []byte
	record.SetNewest(
		chunk.Stream,
		s.newestOffset,
		subtract(s.newestOffset, s.head),
		decodeOutputText(&pending, tail, false),
	)
	held := subtract(s.next, chunk.Offset)
	if held >= uint64(len(chunk.Bytes)) {
		return false
	}
	text := ""
	if leftOut := subtract(chunk.Offset, s.next); leftOut > 0 {
		text = decodeOutputText(
			&s.pending,
			nil,
			true,
		) + fmt.Sprintf(
			"\n[... %d bytes of %s not shown ...]\n",
			leftOut,
			chunk.Stream,
		)
	}
	text += decodeOutputText(&s.pending, chunk.Bytes[held:], false)
	s.next = end
	return record.AppendPageOutput(text)
}

// subtract counts runner output not already held without unsigned underflow.
func subtract(total, held uint64) uint64 {
	if total < held {
		return 0
	}
	return total - held
}

// pushReason appends the runner failure to the command's whole output and page tail.
func pushReason(records *[]host.OutputRecord, reason string) string {
	separator := ""
	for i := len(*records) - 1; i >= 0; i-- {
		record := (*records)[i]
		if record.LeftOut == nil && len(record.Bytes) > 0 {
			if record.Bytes[len(record.Bytes)-1] != '\n' {
				separator = "\n"
			}
			break
		}
	}
	text := separator + reason + "\n"
	*records = append(*records, host.OutputRecord{Stream: core.StreamKind("stderr"), Bytes: []byte(text)})
	return text
}

// finishJob stores edits, retains whole output and updates the shell directory before settlement.
func (e *ShellEnvironment) finishJob(
	ctx context.Context,
	shell core.ShellID,
	command core.CommandID,
	running *runningCommand,
	record *host.CommandRecord,
	job *Job,
	end JobEnd,
	streams [2]receivedStream,
) {
	e.keepJobFiles(ctx, shell, command, record, end)
	received := running.received
	ending := host.Ending{Phase: host.Exited}
	switch end.Status.Kind {
	case host.ProcessNotStarted:
		reason := "bash: " + string(end.Status.SpawnError.Kind)
		if end.Status.SpawnError.Kind == host.OtherSpawnError && end.Status.SpawnError.Detail != nil {
			reason = *end.Status.SpawnError.Detail
		}
		page := pushReason(&received, reason)
		ending.ExitCode = 127
		e.settle(ctx, command, record, ending, host.WholeOutput{Records: received}, nil, page, job)
		return
	case host.ProcessLost:
		missingBytes := subtract(streams[0].length, streams[0].head) + subtract(streams[1].length, streams[1].head)
		output := host.WholeOutput{}
		page := pushReason(&received, end.Status.Reason)
		output.Records = received
		if missingBytes > 0 {
			output.Missing = &host.Missing{Bytes: missingBytes, Reason: "lost with the Host's connection"}
			page += output.Missing.Line() + "\n"
		}
		ending.ExitCode = 127
		e.settle(ctx, command, record, ending, output, nil, page, job)
		return
	case host.ProcessExited:
		ending.ExitCode = end.Status.ExitCode
	case host.ProcessSignalled:
		ending.ExitCode = 128
		if end.Status.Signal == "SIGTERM" || end.Status.Signal == "SIGKILL" {
			ending.ExitCode = 130
		}
	}
	e.finishJobOutput(ctx, command, running, record, job, end, streams, ending)
}

// settle stores output, releases the runner directory, then publishes the command's end.
func (e *ShellEnvironment) settle(
	ctx context.Context,
	command core.CommandID,
	record *host.CommandRecord,
	ending host.Ending,
	output host.WholeOutput,
	binary *host.BinaryOutput,
	page string,
	job *Job,
) {
	if e.options.Keeper != nil {
		if err := e.options.Keeper.KeepOutput(ctx, command, output); err != nil {
			slog.Warn("a command's output was not stored: "+err.Error(), "command", command)
		}
	}
	if job != nil {
		if err := job.Release(ctx); err != nil {
			slog.Debug("job release not sent: "+err.Error(), "job", job.ID())
		}
	}
	if record.Settle(ending, &output, binary, page) {
		e.options.Feed.Changed(record)
	}
}

// keepJobFiles retains edits before updating the shell directory for settlement.
func (e *ShellEnvironment) keepJobFiles(
	ctx context.Context,
	shell core.ShellID,
	command core.CommandID,
	record *host.CommandRecord,
	end JobEnd,
) {
	if len(end.Files) > 0 {
		var files []core.EditedFile
		if e.options.Keeper != nil {
			retained, err := e.options.Keeper.Retain(ctx, command, end.Files)
			files = retained
			if err != nil {
				slog.Warn("an edit's copies were not stored: "+err.Error(), "command", command)
			}
		}
		if e.options.Keeper == nil {
			for _, file := range end.Files {
				files = append(files, EditedFile(file, func(int) *core.EditCopies { return nil }))
			}
		}
		record.SetFiles(host.EditedFiles{Files: files, Truncated: end.FilesTruncated})
	}
	if end.CWD != nil {
		e.mu.Lock()
		if state := e.shells[shell]; state != nil {
			state.cwd = *end.CWD
		}
		e.mu.Unlock()
	}
}

// completeJobOutput reads retained output when views omit bytes, keeping received tails on failure.
func completeJobOutput(
	ctx context.Context,
	command core.CommandID,
	running *runningCommand,
	job *Job,
	lengths runnerwire.OutputLengths,
	streams [2]receivedStream,
) (host.WholeOutput, string, bool) {
	unreceived := subtract(lengths.StdoutBytes, streams[0].head) + subtract(lengths.StderrBytes, streams[1].head)
	output := host.WholeOutput{Records: running.received}
	page := ""
	read := false
	if unreceived > 0 {
		whole, err := job.ReadOutput(ctx)
		if err == nil {
			output = whole
			read = true
		} else {
			slog.Warn("could not read the command's kept output: "+err.Error(), "command", command)
			output, page = partialJobOutput(output, streams, unreceived, err)
		}
	}
	return output, page, read
}

// partialJobOutput appends received tails and reports bytes missing after a failed retained-output read.
func partialJobOutput(
	output host.WholeOutput,
	streams [2]receivedStream,
	unreceived uint64,
	err error,
) (host.WholeOutput, string) {
	page := ""
	var newest []host.OutputRecord
	var leftOut, kept uint64
	for i, stream := range streams {
		if len(stream.newest) == 0 {
			continue
		}
		gap := subtract(stream.newestOffset, stream.head)
		leftOut += gap
		kept += gap + uint64(len(stream.newest))
		kind := core.StreamKind("stdout")
		if i == 1 {
			kind = core.StreamKind("stderr")
		}
		newest = append(newest, host.OutputRecord{Stream: kind, Bytes: stream.newest})
	}
	if len(newest) > 0 {
		output.Records = append(output.Records, host.OutputRecord{LeftOut: new(leftOut)})
		output.Records = append(output.Records, newest...)
	}
	if missing := subtract(unreceived, kept); missing > 0 {
		output.Missing = &host.Missing{Bytes: missing, Reason: "not read from the Host: " + err.Error()}
		page = output.Missing.Line() + "\n"
	}
	return output, page
}

// finishJobOutput builds the final output view and settles an exited or signalled command.
func (e *ShellEnvironment) finishJobOutput(
	ctx context.Context,
	command core.CommandID,
	running *runningCommand,
	record *host.CommandRecord,
	job *Job,
	end JobEnd,
	streams [2]receivedStream,
	ending host.Ending,
) {
	lengths := runnerwire.OutputLengths{StdoutBytes: streams[0].head, StderrBytes: streams[1].head}
	if end.Output != nil {
		lengths = *end.Output
	}
	output, page, read := completeJobOutput(ctx, command, running, job, lengths, streams)
	binary := output.BinaryStdout(lengths.StdoutBytes, e.options.BinaryLimit)
	var binaryLength *uint64
	if binary != nil {
		binaryLength = new(binary.Info.TotalBytes)
	}
	if read {
		page = output.Text(host.Both, binaryLength, host.Seen{}).Display()
	} else if binaryLength != nil {
		page = host.BinaryLine(*binaryLength) + "\n" + page
	}
	running.mu.Lock()
	aborted := running.aborted
	running.mu.Unlock()
	if aborted {
		ending.Phase = host.Aborted
	}
	record.Grew(core.StreamKind("stdout"), lengths.StdoutBytes)
	record.Grew(core.StreamKind("stderr"), lengths.StderrBytes)
	e.settle(ctx, command, record, ending, output, binary, page, job)
}
