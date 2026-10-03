package remotehost_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/remotehost/testdata/fixture"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// nativeFixture loads the programtest-built package after runner availability was checked.
func nativeFixture(t *testing.T) *remotehosttest.NativeFixture {
	t.Helper()
	native, err := remotehosttest.LoadNativeFixture(t.Context())
	requirePipe(t, err)
	return native
}

// serviceRequest names a user invocation with no environment inherited from a shell.
func serviceRequest(f *remotehosttest.RunnerFixture, n *remotehosttest.NativeFixture, operation string, args json.RawMessage) remotehost.ServiceRequest {
	context := hosttest.CommandContext()
	context.Caller = &commandwire.UserCaller{}
	request := remotehost.ServiceRequest{Context: context, Package: n.Descriptor, Operation: operation, Args: args, CWD: f.Home(), Resolver: n.Resolver()}
	if args != nil {
		request.JSON = new(true)
	}
	return request
}

// openService gives the page ownership of both pipe ends and the service lifetime.
func openService(t *testing.T, f *remotehosttest.RunnerFixture, request remotehost.ServiceRequest) (*remotehost.Pipe, *remotehost.Pipe, *remotehost.ServiceStream, error) {
	t.Helper()
	input := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	output := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	stream, err := f.Host().OpenService(t.Context(), request, input.WireRef(), output.WireRef())
	if err != nil {
		input.Fail(err.Error())
		output.Fail(err.Error())
		return nil, nil, nil, err
	}
	t.Cleanup(stream.Close)
	return input, output, stream, nil
}

func TestRunnerServiceStreamCarriesBytesAndEndsWithInvocation(t *testing.T) {
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Tap: tap.input})
	native := nativeFixture(t)
	input, output, stream, err := openService(t, f, serviceRequest(f, native, "where", nil))
	requirePipe(t, err)
	writer, err := input.Writer()
	requirePipe(t, err)
	writer.End()
	reader, err := output.Reader()
	requirePipe(t, err)
	answer, err := collectPipe(t.Context(), reader)
	requirePipe(t, err)
	requirePipe(t, reader.Close(t.Context()))
	report, err := fixture.DecodeWhereReport(answer)
	requirePipe(t, err)
	expected := serviceRequest(f, native, "where", nil).Context
	if report.Label != nil || report.Value != nil || report.CWD != f.Home() || !reflect.DeepEqual(report.Context, expected) {
		t.Fatal(report)
	}
	end, err := stream.Done(t.Context())
	requirePipe(t, err)
	if end.ExitCode != 0 {
		t.Fatal(end)
	}
	stream.Close()
	tap.find(t, func(message runnerwire.Outbound) bool {
		request, ok := message.(*runnerwire.ArtifactResolve)
		if !ok {
			return false
		}
		_, ok = request.Owner.(*runnerwire.StreamArtifactOwner)
		return ok
	})
	input, output, stream, err = openService(t, f, serviceRequest(f, native, "echo", nil))
	requirePipe(t, err)
	payload := patternedBytes(1024 * 1024)
	writer, err = input.Writer()
	requirePipe(t, err)
	reader, err = output.Reader()
	requirePipe(t, err)
	fed := make(chan error, 1)
	go func() {
		err := writer.Write(t.Context(), payload)
		if err == nil {
			writer.End()
		} else {
			writer.Fail(err.Error())
		}
		fed <- err
	}()
	echoed, err := collectPipe(t.Context(), reader)
	requirePipe(t, err)
	requirePipe(t, reader.Close(t.Context()))
	requirePipe(t, <-fed)
	if !bytes.Equal(echoed, payload) {
		t.Fatal("service echo changed")
	}
	requirePipe(t, output.Done(t.Context()))
	end, err = stream.Done(t.Context())
	requirePipe(t, err)
	if end.ExitCode != 0 {
		t.Fatal(end)
	}
	stream.Close()
	_, _, _, err = openService(t, f, serviceRequest(f, native, "missing", nil))
	requireHostCode(t, err, "unknown_operation")
	input, output, stream, err = openService(t, f, serviceRequest(f, native, "echo", nil))
	requirePipe(t, err)
	writer, err = input.Writer()
	requirePipe(t, err)
	requirePipe(t, writer.Write(t.Context(), []byte("before the page left")))
	reader, err = output.Reader()
	requirePipe(t, err)
	f.Pipes().Fail(input.ID(), "page closed")
	if _, err = collectPipe(t.Context(), reader); err == nil {
		t.Fatal("page cancellation did not fail output")
	}
	requirePipe(t, reader.Close(t.Context()))
	if tap.pipeDone(t, input.ID()).Ok || tap.pipeDone(t, output.ID()).Ok {
		t.Fatal("page cancellation reported success")
	}
	stream.Close()
}

func TestRunnerOneShotCompletionAndLoggedStreamWords(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	native := nativeFixture(t)
	h := f.Host()
	call := func(operation string, args json.RawMessage) ([]byte, error) {
		return h.CallService(t.Context(), serviceRequest(f, native, operation, args), nil, 64*1024)
	}
	args, err := (fixture.WhereArgs{Label: new("from the user")}).MarshalJSON()
	requirePipe(t, err)
	answer, err := call("where", args)
	requirePipe(t, err)
	report, err := fixture.DecodeWhereReport(answer)
	requirePipe(t, err)
	if report.Label == nil || *report.Label != "from the user" {
		t.Fatal(report)
	}
	if _, ok := report.Context.Caller.(*commandwire.UserCaller); !ok {
		t.Fatal(report.Context)
	}
	_, err = call("result", nil)
	var failure *remotehost.ServiceCallError
	if !errors.As(err, &failure) || failure.Kind != remotehost.ServiceExited || failure.ExitCode != 17 || failure.Stderr != "command diagnostic" || string(failure.Stdout) != "command output" {
		t.Fatal(err)
	}
	_, err = call("retain", nil)
	requirePipe(t, err)
	requirePipe(t, h.ReleaseConversation(t.Context(), report.Context.Conversation))
	_, _, _, err = openService(t, f, serviceRequest(f, native, "missing", nil))
	if err == nil {
		t.Fatal("missing operation admitted")
	}
	page := logMatching(t, h, new(uint64(0)), nil, func(page remotehost.LogPage) bool {
		return logCount(page, "stream:result ended") > 0 && logCount(page, "stream:missing refused (unknown_operation): demicodes.runner-test has no operation missing") > 0
	})
	for _, want := range []struct{ source, text string }{{"stream:result", "command diagnostic"}, {"runner", "stream:result opened"}, {"runner", "stream:result ended"}, {"runner", "stream:missing refused (unknown_operation): demicodes.runner-test has no operation missing"}} {
		found := false
		for _, line := range page.Lines {
			if line.Source == want.source && line.ConversationID != nil && *line.ConversationID == report.Context.Conversation && line.Text == want.text {
				found = true
			}
		}
		if !found {
			t.Fatal("missing log", want, page)
		}
	}
	started := false
	for _, line := range page.Lines {
		if line.Source == "runner" && strings.HasPrefix(line.Text, "service demicodes.runner-test started (pid ") && strings.HasSuffix(line.Text, ")") {
			pid := strings.TrimSuffix(strings.TrimPrefix(line.Text, "service demicodes.runner-test started (pid "), ")")
			_, err := strconv.ParseUint(pid, 10, 32)
			started = started || err == nil
		}
	}
	if !started {
		t.Fatal("missing service start log")
	}
}

func TestRunnerUserCallsReuseLastBoundServiceRelease(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	first := nativeFixture(t)
	binary, err := remotehosttest.NativeFixtureBinary(t.Context())
	requirePipe(t, err)
	executable, err := os.ReadFile(binary)
	requirePipe(t, err)
	path := filepath.Join(t.TempDir(), "fixture")
	requirePipe(t, os.WriteFile(path, append(executable, 0), 0700))
	second, err := remotehosttest.NewNativeFixture(t.Context(), first.Descriptor.ID, path, first.Descriptor.Operations)
	requirePipe(t, err)
	call := func(native *remotehosttest.NativeFixture) {
		t.Helper()
		_, err := f.Host().CallService(t.Context(), serviceRequest(f, native, "where", nil), nil, 64*1024)
		requirePipe(t, err)
	}
	call(first)
	call(first)
	input, output, stream, err := openService(t, f, serviceRequest(f, first, "echo", nil))
	requirePipe(t, err)
	call(second)
	writer, err := input.Writer()
	requirePipe(t, err)
	writer.End()
	reader, err := output.Reader()
	requirePipe(t, err)
	data, err := collectPipe(t.Context(), reader)
	requirePipe(t, err)
	requirePipe(t, reader.Close(context.Background()))
	if len(data) != 0 {
		t.Fatal(data)
	}
	end, err := stream.Done(t.Context())
	requirePipe(t, err)
	if end.ExitCode != 0 {
		t.Fatal(end)
	}
	stream.Close()
	call(second)
	stopped := "service demicodes.runner-test stopped"
	page := logMatching(t, f.Host(), new(uint64(0)), nil, func(page remotehost.LogPage) bool {
		return logCount(page, "stream:where ended") == 4 && logCount(page, stopped) == 1
	})
	starts := 0
	stopsAt := -1
	echoAt := -1
	for i, line := range page.Lines {
		if strings.HasPrefix(line.Text, "service demicodes.runner-test started ") {
			starts++
		}
		if line.Text == "service demicodes.runner-test holds no lease or conversation and stops" {
			stopsAt = i
		}
		if line.Text == "stream:echo ended" {
			echoAt = i
		}
	}
	if starts != 2 || logCount(page, stopped) != 1 || echoAt < 0 || stopsAt <= echoAt {
		t.Fatal(page)
	}
}
