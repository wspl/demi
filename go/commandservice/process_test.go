package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/fixture"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// The tests of this file run a service as a child process, over the pipes of
// its standard input and output: what the runner does with a native program.
// They cover what only a process shows (its transport, its exit status) and the
// client against a real peer. The service is the Go fixture program, which this
// test binary is when started with asService set, or, when DEMI_RUST_FIXTURE
// names the executable of the Rust fixture, that one: the same tests then check
// the client against the other implementation. The fixture's own behaviors are
// the Rust judge's to check (runner, host_remote and backend tests).

// asService makes the test binary a service of its own when it is started with
// it set: the fixture program, or a probe of what its processes inherit.
const asService = "DEMI_COMMANDSERVICE_TEST_AS"

func TestMain(m *testing.M) {
	switch os.Getenv(asService) {
	case "fixture":
		fixture.Main()
	case "probe":
		serveProbe()
	}
	os.Exit(m.Run())
}

// probe is a service whose one operation starts a process, to show what that
// process inherits.
type probe struct{}

func (probe) Operations() []string { return []string{"child"} }

// Invoke starts a shell that signals itself with SIGPIPE. The shell ends of it
// unless it inherited the signal as ignored; it then says that it lived.
func (probe) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	out, err := exec.Command("sh", "-c", "kill -s PIPE $$; echo survived").Output()
	if err != nil {
		return commandservice.Completion{}, err
	}
	_, err = call.Stdout.Write(out)
	return commandservice.Completion{}, err
}

func serveProbe() {
	if err := commandservice.ServeStdio(context.Background(), probe{}); err != nil {
		fmt.Fprintf(os.Stderr, "probe: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}

// startGo starts the Go fixture program, or the probe, as a child process.
func startGo(t *testing.T, mode string) *servicetest.Process {
	t.Helper()
	// A test binary built with the race detector waits a second before it
	// exits, to let other threads report races; the service has none to wait
	// for.
	return servicetest.StartProcess(testContext(t), t, os.Args[0], nil, []string{asService + "=" + mode, "GORACE=atexit_sleep_ms=0"})
}

// startTarget starts the service the tests of the client run against: the
// Rust fixture when DEMI_RUST_FIXTURE names it, the Go fixture program if not.
func startTarget(t *testing.T) *servicetest.Process {
	t.Helper()
	if rust := os.Getenv("DEMI_RUST_FIXTURE"); rust != "" {
		return servicetest.StartProcess(testContext(t), t, rust, nil, nil)
	}
	return startGo(t, "fixture")
}

// exits waits for the process to exit and returns its exit status.
func exits(t *testing.T, process *servicetest.Process) int {
	t.Helper()
	state, err := process.Wait(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	return state.ExitCode()
}

func TestAServiceProcessAnswersItsCatalogAndExitsAfterAShutdown(t *testing.T) {
	process := startTarget(t)
	info, err := process.Client.Info(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	if info.ProtocolVersion != 1 || len(info.Operations) == 0 {
		t.Errorf("catalog = %+v", info)
	}
	if err := process.Client.Shutdown(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if status := exits(t, process); status != 0 {
		t.Errorf("exit status %d, standard error %q", status, process.Stderr())
	}
}

// pattern returns n bytes that differ from block to block and are not text.
func pattern(n int) []byte {
	data := make([]byte, n)
	for i := range data {
		data[i] = byte(i*31 + i>>8)
	}
	return data
}

func TestBinaryInputEchoesThroughTheFlowControlOfARealConnection(t *testing.T) {
	process := startTarget(t)
	// More than a stream window holds and than one chunk, so the pulls, the
	// chunk boundaries and the flow control all work.
	input := pattern(1 << 20)
	echoed := run(t, process.Client, invocation("echo"), input)
	if !bytes.Equal(echoed.stdout, input) || echoed.completion.ExitCode != 0 {
		t.Errorf("echoed %d bytes, want %d, completion %+v", len(echoed.stdout), len(input), echoed.completion)
	}
	first := run(t, process.Client, invocation("first"), input)
	if !bytes.Equal(first.stdout, input[:64*1024]) {
		t.Errorf("first echoed %d bytes, want one chunk of 65536", len(first.stdout))
	}
}

func TestMetadataLargerThanAWindowAndOutputLargerThanARecordRoundTrip(t *testing.T) {
	process := startTarget(t)
	// The label is a few times a stream window, and more than one record of
	// output when `where` prints it back.
	label := strings.Repeat("λ", 100*1024)
	call := invocation("where")
	call.Args = []byte(`{"label":"` + label + `"}`)
	got := run(t, process.Client, call, nil)
	if !strings.Contains(string(got.stdout), `"label":"`+label+`"`) {
		t.Errorf("the label did not come back: %d bytes of output", len(got.stdout))
	}
}

func TestCancellingACallThatWaitsForInputKeepsTheService(t *testing.T) {
	process := startTarget(t)
	// `echo` asks for input and waits for it: the caller's request is open and
	// idle, where a cancellation has to reach the service through the transport
	// that waits for the next chunk.
	waiting, err := process.Client.Invoke(testContext(t), invocation("echo"))
	if err != nil {
		t.Fatal(err)
	}
	if record, err := waiting.Next(); err != nil || record.Kind != commandservice.RecordInputPull {
		t.Fatalf("first record %+v, %v; want an input pull", record, err)
	}
	waiting.Cancel()
	if _, err := waiting.Next(); !errors.Is(err, commandservice.ErrCancelled) {
		t.Errorf("after the cancel: error = %v, want ErrCancelled", err)
	}
	if echoed := run(t, process.Client, invocation("echo"), []byte("still here")); string(echoed.stdout) != "still here" {
		t.Errorf("the next invocation echoed %q", echoed.stdout)
	}
}

// The fixture's `where` prints what an invocation carries as JSON, its members
// in the order the Rust fixture writes them, and the two fixtures write the same
// bytes.
func TestWhereAnswersTheBytesOfTheRustFixture(t *testing.T) {
	process := startTarget(t)
	call := invocation("where")
	call.Args = []byte(`{"label":"A"}`)
	call.Env = map[string]string{"PROBE": "alpha", "OTHER": "ignored"}
	call.Cwd = "/work"
	call.Context.Caller = commandservice.UserCaller()
	want := `{"label":"A","context":{"conversation":"conversation","caller":{"kind":"user"},"locale":{"timeZone":"UTC","languages":["en-US"]}},"cwd":"/work","value":"alpha"}`
	if got := run(t, process.Client, call, nil); string(got.stdout) != want {
		t.Errorf("where = %s\nwant    %s", got.stdout, want)
	}
	// Without a label or a probe, both are null.
	bare := run(t, process.Client, invocation("where"), nil)
	want = `{"label":null,"context":{"conversation":"conversation","caller":{"kind":"agent","number":1},"locale":{"timeZone":"UTC","languages":["en-US"]}},"cwd":"/tmp","value":null}`
	if string(bare.stdout) != want {
		t.Errorf("where = %s\nwant    %s", bare.stdout, want)
	}
}

// A conversation request is metadata only: the client sends it with the end of
// the request, and the service answers from it.
func TestConversationsAreHeldAndReleasedThroughTheConversationEndpoint(t *testing.T) {
	process := startTarget(t)
	held := func() string {
		answer, completion := conversation(t, process.Client, status())
		if completion.ExitCode != 0 {
			t.Fatalf("status completion = %+v", completion)
		}
		return answer
	}
	if got := held(); got != `{"conversations":[]}` {
		t.Fatalf("status of a new service = %s", got)
	}
	for _, name := range []string{"two", "one"} {
		call := invocation("retain")
		call.Context.Conversation = name
		run(t, process.Client, call, nil)
	}
	if got := held(); got != `{"conversations":["one","two"]}` {
		t.Errorf("status = %s", got)
	}
	if answer, completion := conversation(t, process.Client, release("one")); answer != "{}" || completion.ExitCode != 0 {
		t.Errorf("release = %s %+v", answer, completion)
	}
	if got := held(); got != `{"conversations":["two"]}` {
		t.Errorf("status after the release = %s", got)
	}
}

func TestNumbersAreDrawnThroughTheNumbersStreamAndItEndsWithTheShutdown(t *testing.T) {
	process := startTarget(t)
	answering, err := servicetest.AnswerNumbers(testContext(t), process.Client)
	if err != nil {
		t.Fatal(err)
	}
	draw := func(conversation, count string) string {
		call := invocation("number")
		call.Context.Conversation = conversation
		call.Args = []byte(`{"count":` + count + `}`)
		got := run(t, process.Client, call, nil)
		if got.completion.ExitCode != 0 {
			t.Fatalf("number %s: %+v %s", count, got.completion, got.stderr)
		}
		return string(got.stdout)
	}
	if got := draw("one", "4"); got != `{"first":1}` {
		t.Errorf("first draw = %s", got)
	}
	// Each conversation's numbers go on from its last draw.
	if got := draw("one", "1"); got != `{"first":5}` {
		t.Errorf("second draw = %s", got)
	}
	if got := draw("two", "1"); got != `{"first":1}` {
		t.Errorf("draw of another conversation = %s", got)
	}
	// Shutdown completes the stream, so the connection drains and the service
	// ends.
	if err := process.Client.Shutdown(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if err := <-answering; err != nil {
		t.Errorf("the numbers stream ended with %v", err)
	}
	if status := exits(t, process); status != 0 {
		t.Errorf("exit status %d: %s", status, process.Stderr())
	}
}

// The tests below are of the Go program alone: what its exit says.

// A process's connection is its standard input and output: its peer closing its
// side is the end of the service's input, and the service ends.
func TestTheGoServiceExitsWhenItsPeerClosesItsSide(t *testing.T) {
	process := startGo(t, "fixture")
	if _, err := process.Client.Info(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if err := process.CloseInput(); err != nil {
		t.Fatal(err)
	}
	if status := exits(t, process); status != 0 {
		t.Errorf("exit status %d, standard error %q", status, process.Stderr())
	}
}

// A conversation release that fails leaves the service faulty: the process says
// why on standard error and exits with status 1, so that its runner retires it.
func TestAFailedReleaseEndsTheGoServiceWithItsReasonOnStandardError(t *testing.T) {
	process := startGo(t, "fixture")
	// The service may end before its answer reaches the client.
	if stream, err := process.Client.Conversation(testContext(t), release("fail")); err == nil {
		_, _ = commandservice.Exchange(testContext(t), stream, bytes.NewReader(nil), &bytes.Buffer{}, &bytes.Buffer{})
	}
	status := exits(t, process)
	want := "fixture: conversation cleanup failed; retire the service process: release exited with status 1: command_failed: fixture cleanup failed"
	if status != 1 || !strings.Contains(process.Stderr(), want) {
		t.Errorf("exit status %d, standard error %q; want 1 and %q", status, process.Stderr(), want)
	}
}

// A service whose runner closed its end of the pipe of its standard output finds
// out when it writes to it: the write fails, and the service ends with an error
// status, where a signal that killed it would show nothing of why.
func TestAServiceWhoseOutputPipeIsClosedEndsWithAnErrorStatusNotASignal(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGPIPE is a Unix signal")
	}
	// Nobody reads the pipe of the service's standard output, so its first write
	// fails; its standard input stays open, so that nothing else ends it.
	stdoutReader, stdoutWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := stdoutReader.Close(); err != nil {
		t.Fatal(err)
	}
	defer stdoutWriter.Close()
	stdinReader, stdinWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer stdinReader.Close()
	defer stdinWriter.Close()
	var stderr bytes.Buffer
	service := exec.CommandContext(testContext(t), os.Args[0])
	service.Env = append(os.Environ(), asService+"=fixture", "GORACE=atexit_sleep_ms=0")
	service.Stdin = stdinReader
	service.Stdout = stdoutWriter
	service.Stderr = &stderr
	if err := service.Start(); err != nil {
		t.Fatal(err)
	}
	// A client's preface and its settings let the service go on to write.
	if _, err := stdinWriter.Write(append([]byte(clientPreface), h2Frame{kind: frameSettings}.bytes()...)); err != nil {
		t.Fatal(err)
	}
	var exit *exec.ExitError
	if err := service.Wait(); !errors.As(err, &exit) || exit.ExitCode() != 1 {
		t.Errorf("the service ended with %v, standard error %q; want exit status 1", err, stderr.String())
	}
}

// The processes a handler starts inherit the signals their parent caught as
// defaults, and the ones it ignored as ignored: a service that ignored SIGPIPE
// would hand that to every command it runs.
func TestTheProcessesAServiceStartsKeepSIGPIPEAtItsDefault(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGPIPE is a Unix signal")
	}
	process := startGo(t, "probe")
	got := run(t, process.Client, invocation("child"), nil)
	if got.completion.ExitCode != 1 || got.completion.Error == nil || !strings.Contains(got.completion.Error.Message, "broken pipe") {
		t.Errorf("the shell that signaled itself completed with %+v, output %q; want it ended by SIGPIPE", got.completion, got.stdout)
	}
}
