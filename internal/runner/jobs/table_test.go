package jobs_test

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// playedShell runs event-controlled job scenarios without importing a shell implementation.
type playedShell struct {
	run func(context.Context, process.JobStart, chan<- process.OutputChunk, <-chan process.Input) (process.Exit, *string)
}

func (*playedShell) BuiltinNames() map[string]struct{} { return map[string]struct{}{} }
func (s *playedShell) Start(ctx context.Context, start process.JobStart) (process.ShellJob, error) {
	ctx, cancel := context.WithCancel(ctx)
	job := &playedJob{
		ctx:    ctx,
		cancel: cancel,
		input:  make(chan process.Input),
		output: make(chan process.OutputChunk),
		done:   make(chan struct{}),
	}
	go func() {
		defer close(job.done)
		defer close(job.output)
		job.exit, job.cwd = s.run(ctx, start, job.output, job.input)
	}()
	return job, nil
}

type playedJob struct {
	ctx    context.Context
	cancel context.CancelFunc
	input  chan process.Input
	output chan process.OutputChunk
	done   chan struct{}
	exit   process.Exit
	cwd    *string
}

func (j *playedJob) Input() chan<- process.Input        { return j.input }
func (j *playedJob) Output() <-chan process.OutputChunk { return j.output }
func (j *playedJob) Signal(runnerwire.Signal) error {
	j.cancel()
	return nil
}
func (j *playedJob) Cancel()           { j.cancel() }
func (j *playedJob) IsCancelled() bool { return j.ctx.Err() != nil }
func (j *playedJob) Wait(ctx context.Context) (process.Exit, *string, error) {
	select {
	case <-j.done:
	case <-ctx.Done():
		j.cancel()
		<-j.done
	}
	return j.exit, j.cwd, nil
}

func printJob(
	ctx context.Context,
	output chan<- process.OutputChunk,
	stream runnerwire.OutputStream,
	data []byte,
) bool {
	for len(data) > 0 {
		n := min(len(data), 65536)
		select {
		case <-ctx.Done():
			return false
		case output <- process.OutputChunk{Stream: stream, Bytes: append([]byte{}, data[:n]...)}:
		}
		data = data[n:]
	}
	return true
}

func successfulExit() process.Exit {
	code := int32(0)
	return process.Exit{Code: &code}
}

func newTable(t *testing.T, capacity int, shell *playedShell) (*jobs.Table, <-chan []byte, *jobs.Directories, string) {
	t.Helper()
	root := t.TempDir()
	directories := jobs.OpenDirectories(t.Context(), filepath.Join(root, "jobs"))
	output := make(chan []byte, capacity)
	table := jobs.NewTable(t.Context(), jobs.Config{Output: output, Directories: directories, Shell: shell})
	t.Cleanup(func() {
		if err := table.Close(context.Background()); err != nil {
			t.Error(err)
		}
		directories.Clear(context.Background())
	})
	return table, output, directories, root
}

func startJob(t *testing.T, table *jobs.Table, root string) {
	t.Helper()
	if err := table.Start(
		jobs.TaskSpec{ID: "job", Cwd: root, Env: map[string]string{}, Command: &jobs.ShellCommand{Script: "fixture"}},
	); err != nil {
		t.Fatal(err)
	}
}

func reply(t *testing.T, output <-chan []byte) runnerwire.Outbound {
	t.Helper()
	message, err := runnerwire.DecodeOutbound(<-output)
	if err != nil {
		t.Fatal(err)
	}
	return message
}

func kept(t *testing.T, directories *jobs.Directories) []runnerwire.KeptRecord {
	t.Helper()
	reader, ok := directories.Output("job")
	if !ok {
		t.Fatal("missing job output")
	}
	snapshot, err := reader.Snapshot(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = snapshot.Close() }()
	data, err := io.ReadAll(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	records, err := runnerwire.DecodeRecords(data)
	if err != nil {
		t.Fatal(err)
	}
	return records
}

func streamBytes(records []runnerwire.KeptRecord, stream runnerwire.OutputStream) []byte {
	var bytes []byte
	for _, record := range records {
		if output, ok := record.(*runnerwire.KeptOutput); ok && output.Stream == stream {
			bytes = append(bytes, output.Bytes...)
		}
	}
	return bytes
}

func TestShellJobKeepsReadsAndSendsViews(t *testing.T) {
	stdout := bytes.Repeat([]byte{'0'}, 60000)
	stderr := bytes.Repeat([]byte{'1'}, 40000)
	table, output, directories, root := newTable(
		t,
		8,
		&playedShell{
			run: func(
				ctx context.Context,
				start process.JobStart,
				out chan<- process.OutputChunk,
				_ <-chan process.Input,
			) (process.Exit, *string) {
				if start.Script != "fixture" {
					t.Error("script changed")
				}
				printJob(ctx, out, runnerwire.Stdout, stdout)
				printJob(ctx, out, runnerwire.Stderr, stderr)
				cwd := filepath.Join(start.Cwd, "child")
				return successfulExit(), &cwd
			},
		},
	)
	startJob(t, table, root)
	heads := map[runnerwire.OutputStream][]byte{}
	newest := map[runnerwire.OutputStream]*runnerwire.JobOutput{}
	for {
		message := reply(t, output)
		if out, ok := message.(*runnerwire.JobOutput); ok {
			printed := stdout
			if out.Stream == runnerwire.Stderr {
				printed = stderr
			}
			if out.Offset < runnerwire.JobViewBytes {
				if out.Offset != uint64(len(heads[out.Stream])) {
					t.Fatal("out-of-order view")
				}
				heads[out.Stream] = append(heads[out.Stream], out.Bytes...)
			} else {
				if len(out.Bytes) > runnerwire.JobViewBytes ||
					!bytes.Equal(out.Bytes, printed[out.Offset:int(out.Offset)+len(out.Bytes)]) {
					t.Fatal("invalid newest view")
				}
				newest[out.Stream] = out
			}
			continue
		}
		exit, ok := message.(*runnerwire.JobExit)
		if !ok {
			t.Fatalf("unexpected %T", message)
		}
		if exit.ExitCode == nil || *exit.ExitCode != 0 || exit.CWD == nil ||
			*exit.CWD != filepath.Join(root, "child") ||
			exit.Output == nil ||
			exit.Output.StdoutBytes != 60000 ||
			exit.Output.StderrBytes != 40000 {
			t.Fatalf("exit %+v", exit)
		}
		break
	}
	for stream, printed := range map[runnerwire.OutputStream][]byte{runnerwire.Stdout: stdout, runnerwire.Stderr: stderr} {
		if !bytes.Equal(heads[stream], printed[:runnerwire.JobViewBytes]) {
			t.Fatal("initial view changed")
		}
		last := newest[stream]
		if last == nil || len(last.Bytes) != runnerwire.JobViewBytes ||
			last.Offset+uint64(len(last.Bytes)) != uint64(len(printed)) {
			t.Fatal("missing last view")
		}
	}
	records := kept(t, directories)
	if !bytes.Equal(streamBytes(records, runnerwire.Stdout), stdout) ||
		!bytes.Equal(streamBytes(records, runnerwire.Stderr), stderr) {
		t.Fatal("kept output differs")
	}
	if err := table.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if table.Len() != 0 {
		t.Fatal("tasks remain after shutdown")
	}
	directories.Release(t.Context(), "job")
	if _, ok := directories.Output("job"); ok {
		t.Fatal("released job retained")
	}
	entries, err := os.ReadDir(directories.Root())
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if entry.IsDir() {
			t.Fatal("released directory retained")
		}
	}
}

func TestEndlessJobKeepsBoundedHeadAndTail(t *testing.T) {
	// Moves 24 MiB through the real job table and disk retention boundary.
	const total = 24 * 1024 * 1024
	data := make([]byte, total)
	pattern := []byte("0123456789\n")
	for i := range data {
		data[i] = pattern[i%len(pattern)]
	}
	table, output, directories, root := newTable(
		t,
		64,
		&playedShell{
			run: func(
				ctx context.Context,
				_ process.JobStart,
				out chan<- process.OutputChunk,
				_ <-chan process.Input,
			) (process.Exit, *string) {
				printJob(ctx, out, runnerwire.Stdout, data)
				printJob(ctx, out, runnerwire.Stdout, []byte("END"))
				return successfulExit(), nil
			},
		},
	)
	startJob(t, table, root)
	for {
		if exit, ok := reply(t, output).(*runnerwire.JobExit); ok {
			if exit.Output == nil || exit.Output.StdoutBytes != total+3 {
				t.Fatalf("exit %+v", exit)
			}
			break
		}
	}
	var onDisk int64
	err := filepath.WalkDir(directories.Root(), func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() && filepath.Base(filepath.Dir(path)) == "output" {
			info, err := entry.Info()
			if err != nil {
				return err
			}
			onDisk += info.Size()
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if onDisk > runnerwire.JobKeptBytes {
		t.Fatalf("kept %d bytes", onDisk)
	}
	records := kept(t, directories)
	gap := -1
	var omitted uint64
	for i, record := range records {
		if left, ok := record.(*runnerwire.KeptLeftOut); ok {
			gap = i
			omitted = left.Bytes
			break
		}
	}
	if gap < 0 {
		t.Fatal("missing gap")
	}
	first, last := streamBytes(records[:gap], runnerwire.Stdout), streamBytes(records[gap+1:], runnerwire.Stdout)
	if !bytes.HasPrefix(first, []byte("0123456789\n0123456789\n")) || !bytes.HasSuffix(last, []byte("01END")) ||
		uint64(len(first)+len(last))+omitted != total+3 {
		t.Fatal("head, gap and tail do not reconstruct length")
	}
	headBytes := 0
	for _, record := range records[:gap] {
		encoded, err := runnerwire.EncodeRecord(record)
		if err != nil {
			t.Fatal(err)
		}
		headBytes += len(encoded)
	}
	if headBytes > runnerwire.JobKeptPartBytes || headBytes <= runnerwire.JobKeptPartBytes-65536 {
		t.Fatalf("head: %d", headBytes)
	}
}

// Cost: 500 ms of virtual printing time; no wall-clock delays.
func TestFollowedJobSendsNewestOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		printed := make(chan struct{}, 4)
		table, output, _, root := newTable(
			t,
			64,
			&playedShell{
				run: func(
					ctx context.Context,
					_ process.JobStart,
					out chan<- process.OutputChunk,
					input <-chan process.Input,
				) (process.Exit, *string) {
					printJob(ctx, out, runnerwire.Stdout, bytes.Repeat([]byte{'0'}, 60000))
					printed <- struct{}{}
					select {
					case <-ctx.Done():
						return process.Exit{}, nil
					case <-input:
					}
					for line := range 50 {
						printJob(ctx, out, runnerwire.Stdout, []byte(fmt.Sprintf("%05d\n", line)))
						time.Sleep(10 * time.Millisecond)
					}
					printed <- struct{}{}
					select {
					case <-ctx.Done():
						return process.Exit{}, nil
					case <-input:
					}
					printJob(ctx, out, runnerwire.Stdout, []byte(fmt.Sprintf("%020000d", 1)))
					printed <- struct{}{}
					select {
					case <-ctx.Done():
						return process.Exit{}, nil
					case <-input:
					}
					printJob(ctx, out, runnerwire.Stdout, []byte("end"))
					return successfulExit(), nil
				},
			},
		)
		startJob(t, table, root)
		<-printed
		next := func() *runnerwire.JobOutput {
			t.Helper()
			out, ok := reply(t, output).(*runnerwire.JobOutput)
			if !ok {
				t.Fatal("expected output")
			}
			return out
		}
		var head uint64
		for {
			out := next()
			if out.Offset >= runnerwire.JobViewBytes {
				end := out.Offset + uint64(len(out.Bytes))
				if head != runnerwire.JobViewBytes || end > 60000 ||
					uint64(len(out.Bytes)) != min(uint64(runnerwire.JobViewBytes), end-runnerwire.JobViewBytes) {
					t.Fatalf("unfollowed head %d, growth %+v", head, out)
				}
				break
			}
			if out.Offset != head {
				t.Fatalf("head offset %d, want %d", out.Offset, head)
			}
			head += uint64(len(out.Bytes))
		}
		synctest.Wait()
		id := jobs.WorkID{Kind: jobs.ShellWork, ID: "job"}
		table.Follow(id, true)
		out := next()
		if out.Offset != 60000-runnerwire.JobLiveBytes ||
			!bytes.Equal(out.Bytes, bytes.Repeat([]byte{'0'}, runnerwire.JobLiveBytes)) {
			t.Fatalf("catchup %+v", out)
		}
		at := time.Now()
		if err := table.Input(id, []byte("\n")); err != nil {
			t.Fatal(err)
		}
		end, messages := uint64(60000), 0
		for end < 60300 {
			out = next()
			if out.Offset != end {
				t.Fatalf("live offset %d, want %d", out.Offset, end)
			}
			end += uint64(len(out.Bytes))
			messages++
		}
		intervals := int(time.Since(at) / runnerwire.JobLiveInterval)
		if messages > intervals+2 {
			t.Fatalf("%d messages in %v", messages, time.Since(at))
		}
		<-printed
		table.Follow(id, false)
		synctest.Wait()
		if err := table.Input(id, []byte("\n")); err != nil {
			t.Fatal(err)
		}
		<-printed
		synctest.Wait()
		for len(output) > 0 {
			if out := next(); len(out.Bytes) != 0 {
				t.Fatalf("unfollowed bytes %+v", out)
			}
		}
		table.Follow(id, true)
		out = next()
		newest := append(bytes.Repeat([]byte{'0'}, runnerwire.JobLiveBytes-1), '1')
		if out.Offset != 80300-runnerwire.JobLiveBytes || !bytes.Equal(out.Bytes, newest) {
			t.Fatalf("second catchup %+v", out)
		}
		if err := table.Input(id, []byte("\n")); err != nil {
			t.Fatal(err)
		}
		var last []byte
		for {
			frame := reply(t, output)
			if out, ok := frame.(*runnerwire.JobOutput); ok {
				if len(out.Bytes) > 0 {
					if out.Offset != 80300+uint64(len(last)) {
						t.Fatalf("final offset %+v", out)
					}
					last = append(last, out.Bytes...)
				}
				continue
			}
			exit, ok := frame.(*runnerwire.JobExit)
			if !ok || exit.Output == nil || exit.Output.StdoutBytes != 80303 || string(last) != "end" {
				t.Fatalf("last %q exit %+v", last, frame)
			}
			break
		}
		if err := table.Close(context.Background()); err != nil {
			t.Fatal(err)
		}
	})
}

func TestShutdownWithBlockedOutputConsumer(t *testing.T) {
	table, output, _, root := newTable(
		t,
		1,
		&playedShell{
			run: func(
				ctx context.Context,
				_ process.JobStart,
				out chan<- process.OutputChunk,
				_ <-chan process.Input,
			) (process.Exit, *string) {
				for printJob(ctx, out, runnerwire.Stdout, bytes.Repeat([]byte{'0'}, 4096)) {
				}
				return process.Exit{}, nil
			},
		},
	)
	startJob(t, table, root)
	reply(t, output)
	if err := table.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if table.Len() != 0 {
		t.Fatal("tasks remain after shutdown")
	}
}
