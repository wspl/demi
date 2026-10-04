package commandsdk_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
)

type benchmarkFixture struct{}

func (benchmarkFixture) Operations() []string {
	return []string{"echo", "flood"}
}

func (benchmarkFixture) Invoke(
	ctx context.Context,
	call commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	var chunk []byte
	if call.Request.Operation == "flood" {
		chunk = bytes.Repeat([]byte{0xa5}, 64*1024)
	}
	for {
		if call.Request.Operation != "flood" {
			var err error
			chunk, err = call.Input.Next(ctx)
			if errors.Is(err, io.EOF) {
				return commandproto.Completion{}, nil
			}
			if err != nil {
				return commandproto.Completion{}, err
			}
		}
		if err := call.Output.Stdout(ctx, chunk); err != nil {
			return commandproto.Completion{}, err
		}
	}
}

// TestBenchmarkService is entered only by the benchmark's child process.
func TestBenchmarkService(t *testing.T) {
	if os.Getenv("DEMI_BENCHMARK_SERVICE") != "1" {
		t.Skip("benchmark subprocess only")
	}
	if err := commandsdk.ServeStdio(t.Context(), benchmarkFixture{}); err != nil {
		// Exit status carries failure even when diagnostics cannot be written.
		_, _ = fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	// The testing runner's PASS line must not enter the protocol stdout.
	os.Exit(0)
}

func benchmarkRequest(operation string) (commandproto.Invocation, error) {
	cwd, err := os.Getwd()
	return commandproto.Invocation{
		Operation: operation, InvocationID: "benchmark", Args: []byte(`{}`), Cwd: cwd,
		Env: map[string]string{},
		Context: commandproto.Context{
			Conversation: "conversation", Caller: &commandproto.AgentCaller{Number: 1},
			Locale: commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
		},
	}, err
}

func benchmarkEcho(ctx context.Context, client *commandsdk.Client, size int, delay time.Duration) error {
	request, err := benchmarkRequest("echo")
	if err != nil {
		return err
	}
	input, output, err := client.Invoke(ctx, request)
	if err != nil {
		return err
	}
	defer input.Cancel()
	chunk := bytes.Repeat([]byte{0xa5}, min(size, 64*1024))
	sent, received := 0, 0
	completed := false
	for {
		record, err := output.Next(ctx)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return err
		}
		switch record := record.(type) {
		case commandproto.InputPull:
			if sent < size {
				part := chunk[:min(size-sent, len(chunk))]
				sent += len(part)
				err = input.Write(ctx, part)
			} else {
				err = input.End()
			}
		case commandproto.Stdout:
			for _, value := range record {
				if value != 0xa5 {
					return fmt.Errorf("echo byte got %x, want a5", value)
				}
			}
			received += len(record)
			if delay != 0 {
				err = benchmarkDelay(ctx, delay)
			}
		case commandproto.Completed:
			if record.Completion.ExitCode != 0 {
				return fmt.Errorf("exit code got %d, want 0", record.Completion.ExitCode)
			}
			completed = true
		case commandproto.Stderr:
			return fmt.Errorf("unexpected stderr: %q", record)
		}
		if err != nil {
			return err
		}
	}
	if !completed || received != size {
		return fmt.Errorf("echo got completion=%t bytes=%d, want true %d", completed, received, size)
	}
	return nil
}

// benchmarkDelay models the intentionally slow consumer and unread flood interval.
func benchmarkDelay(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// BenchmarkTransport repeats the complete synthetic workload over child-process
// stdio. Each iteration has 20 warmups, 1000 latency samples, 16 callers doing
// 100 calls each, two 8 MiB transfers, and a blocked flood followed by shutdown.
// The deliberate 2 ms consumer delay and 100 ms flood interval measure real IO.
func BenchmarkTransport(b *testing.B) {
	totals := make(map[string]float64)
	for range b.N {
		metrics, err := benchmarkTransport(b.Context(), b)
		if err != nil {
			b.Fatal(err)
		}
		for name, value := range metrics {
			totals[name] += value
		}
	}
	for name, value := range totals {
		b.ReportMetric(value/float64(b.N), name)
	}
}

func benchmarkTransport(ctx context.Context, b *testing.B) (map[string]float64, error) {
	program, err := os.Executable()
	if err != nil {
		return nil, err
	}
	start := time.Now()
	process, err := commandsdktest.Start(
		ctx,
		b,
		program,
		[]string{"-test.run=^TestBenchmarkService$"},
		[]string{"DEMI_BENCHMARK_SERVICE=1"},
	)
	if err != nil {
		return nil, err
	}
	// Close is idempotent; on failure it kills and reaps the child.
	defer func() {
		_ = process.Close()
	}()
	if _, err := process.Client.Info(ctx); err != nil {
		return nil, err
	}
	metrics := map[string]float64{"startup-us": float64(time.Since(start).Microseconds())}
	stopSampler := benchmarkSampleRSS(ctx, process.PID())
	defer func() {
		if peak := stopSampler(); peak > 0 {
			metrics["peak-combined-rss-bytes"] = float64(peak)
		}
	}()
	if err := benchmarkCalls(ctx, process.Client, metrics); err != nil {
		return nil, err
	}
	for _, delay := range []time.Duration{0, 2 * time.Millisecond} {
		start = time.Now()
		if err := benchmarkEcho(ctx, process.Client, 8*1024*1024, delay); err != nil {
			return nil, err
		}
		elapsed := time.Since(start)
		if delay == 0 {
			metrics["binary-MiB/s"] = 8 / elapsed.Seconds()
		} else {
			metrics["slow-consumer-8mib-ms"] = float64(elapsed.Milliseconds())
		}
	}
	if err := benchmarkBlocked(ctx, process, metrics); err != nil {
		return nil, err
	}
	return metrics, nil
}

func benchmarkCalls(ctx context.Context, client *commandsdk.Client, metrics map[string]float64) error {
	for range 20 {
		if err := benchmarkEcho(ctx, client, 32, 0); err != nil {
			return err
		}
	}
	samples := make([]int64, 1000)
	for i := range samples {
		start := time.Now()
		if err := benchmarkEcho(ctx, client, 32, 0); err != nil {
			return err
		}
		samples[i] = time.Since(start).Microseconds()
	}
	slices.Sort(samples)
	metrics["warm-p50-us"] = float64(samples[500])
	metrics["warm-p95-us"] = float64(samples[950])
	start := time.Now()
	failures := make([]error, 16)
	var workers sync.WaitGroup
	for i := range failures {
		workers.Go(func() {
			for range 100 {
				if err := benchmarkEcho(ctx, client, 32, 0); err != nil {
					failures[i] = err
					return
				}
			}
		})
	}
	workers.Wait()
	metrics["concurrent-calls/s"] = 1600 / time.Since(start).Seconds()
	return errors.Join(failures...)
}

func benchmarkBlocked(ctx context.Context, process *commandsdktest.ServiceProcess, metrics map[string]float64) error {
	request, err := benchmarkRequest("flood")
	if err != nil {
		return err
	}
	input, _, err := process.Client.Invoke(ctx, request)
	if err != nil {
		return err
	}
	defer input.Cancel()
	if err := input.End(); err != nil {
		return err
	}
	if err := benchmarkDelay(ctx, 100*time.Millisecond); err != nil {
		return err
	}
	start := time.Now()
	if err := benchmarkEcho(ctx, process.Client, 32, 0); err != nil {
		return err
	}
	metrics["blocked-peer-call-us"] = float64(time.Since(start).Microseconds())
	start = time.Now()
	input.Cancel()
	if err := process.Shutdown(ctx); err != nil {
		return err
	}
	metrics["cancel-and-shutdown-us"] = float64(time.Since(start).Microseconds())
	return nil
}

// benchmarkRSS reads resident bytes where procfs exposes them; absent data is
// not a zero-memory measurement and therefore produces no benchmark metric.
func benchmarkRSS(pid int) (uint64, bool) {
	data, err := os.ReadFile(fmt.Sprintf("/proc/%d/status", pid))
	if err != nil {
		return 0, false
	}
	for line := range strings.SplitSeq(string(data), "\n") {
		fields := strings.Fields(line)
		if len(fields) >= 2 && fields[0] == "VmRSS:" {
			kb, err := strconv.ParseUint(fields[1], 10, 64)
			return kb * 1024, err == nil
		}
	}
	return 0, false
}

// benchmarkSampleRSS owns and joins the 10 ms combined parent/child sampler.
func benchmarkSampleRSS(ctx context.Context, pid int) func() uint64 {
	ctx, cancel := context.WithCancel(ctx)
	done := make(chan uint64, 1)
	go func() {
		ticker := time.NewTicker(10 * time.Millisecond)
		defer ticker.Stop()
		var peak uint64
		for {
			parent, parentOK := benchmarkRSS(os.Getpid())
			child, childOK := benchmarkRSS(pid)
			if parentOK && childOK {
				peak = max(peak, parent+child)
			}
			select {
			case <-ctx.Done():
				done <- peak
				return
			case <-ticker.C:
			}
		}
	}()
	return func() uint64 {
		cancel()
		return <-done
	}
}
