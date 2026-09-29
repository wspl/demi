package commandservice_test

import (
	"bytes"
	"context"
	"encoding/binary"
	"io"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// The wire's limits on what a service takes and gives, at their edges, in their
// own values: 256 KiB of metadata, 64 KiB of a record or of an input chunk, and
// four records queued for an invocation (docs/execution/native-runtime.md
// § Validation and flow control). The limits of HTTP/2 that the SDK states are
// checked with the frames of a connection (raw_test.go), and the time limits in
// timeouts_test.go.

// metadataBodyFor returns the body of a request that carries call as its
// metadata, behind its length.
func metadataBodyFor(t *testing.T, call commandservice.Invocation) []byte {
	t.Helper()
	document, err := commandservice.Encode(call)
	if err != nil {
		t.Fatal(err)
	}
	return append(binary.BigEndian.AppendUint32(nil, uint32(len(document))), document...)
}

// metadataBody returns the body of a request whose metadata is exactly size
// bytes: an invocation of "noop" with arguments that pad it.
func metadataBody(t *testing.T, size int) []byte {
	t.Helper()
	call := invocation("noop")
	call.Args = []byte(`{"pad":""}`)
	base := metadataBodyFor(t, call)
	call.Args = []byte(`{"pad":"` + strings.Repeat("p", size-(len(base)-4)) + `"}`)
	body := metadataBodyFor(t, call)
	if len(body) != size+4 {
		t.Fatalf("metadata of %d bytes, want %d", len(body)-4, size)
	}
	return body
}

func TestTheServiceTakesMetadataOfExactly256KiBAndRefusesOneByteMore(t *testing.T) {
	server := servicetest.Start(t, operations{"noop": short})
	if err := post(t, server.Client, http.MethodPost, commandservice.InvokePath, metadataBody(t, 256*1024)); err != nil {
		t.Errorf("metadata of 262144 bytes: %v", err)
	}
	// One byte over: JSON allows the blank that takes it there, so the size is
	// all that can be wrong.
	over := metadataBody(t, 256*1024)
	over = append(over, ' ')
	binary.BigEndian.PutUint32(over, 256*1024+1)
	if got := httpStatus(post(t, server.Client, http.MethodPost, commandservice.InvokePath, over)); got != 400 {
		t.Errorf("metadata of 262145 bytes: status %d, want 400", got)
	}
}

// A service writes what a handler writes in records of at most 64 KiB, filling
// each.
func TestOutputLeavesInRecordsOfAtMost64KiB(t *testing.T) {
	server := servicetest.Start(t, operations{"write": func(call *commandservice.Call) (commandservice.Completion, error) {
		_, err := call.Stdout.Write(make([]byte, 200000))
		return commandservice.Completion{}, err
	}})
	stream, err := server.Client.Invoke(testContext(t), invocation("write"))
	if err != nil {
		t.Fatal(err)
	}
	var sizes []int
	for _, record := range readRecords(t, stream) {
		if record.Kind == commandservice.RecordStdout {
			sizes = append(sizes, len(record.Data))
		}
	}
	if len(sizes) != 4 || sizes[0] != 65536 || sizes[1] != 65536 || sizes[2] != 65536 || sizes[3] != 3392 {
		t.Errorf("record sizes %v, want 65536 three times and 3392", sizes)
	}
}

// A service refuses an input chunk over 64 KiB as soon as its length is read.
func TestTheServiceRefusesAnInputChunkOverTheLimit(t *testing.T) {
	server := servicetest.Start(t, operations{"echo": echo})
	body := metadataBodyFor(t, invocation("echo"))
	// The length of a chunk of 65537 bytes, and none of its payload.
	body = binary.BigEndian.AppendUint32(body, 65537)
	ctx, cancel := context.WithCancel(testContext(t))
	defer cancel()
	response, err := server.Client.Send(ctx, http.MethodPost, commandservice.InvokePath, io.NopCloser(bytes.NewReader(body)), int64(len(body)))
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	var completion commandservice.Completion
	reader := commandservice.NewRecordReader(response.Body)
	for {
		record, err := reader.Next()
		if err != nil {
			t.Fatal(err)
		}
		if record.Kind == commandservice.RecordCompletion {
			completion = record.Completion
			break
		}
	}
	if completion.ExitCode != 1 || completion.Error == nil || !strings.Contains(completion.Error.Message, "exceeds limit") {
		t.Errorf("an input chunk of 65537 bytes completed the call with %+v", completion)
	}
}

// An invocation that its caller does not read queues at most four records, and
// holds the one it is sending: its handler writes five records of 64 KiB, and
// blocks at the sixth.
func TestAnInvocationQueuesAtMostFourRecordsBeforeItsHandlerBlocks(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var written atomic.Int32
		server := servicetest.Start(t, operations{"write": func(call *commandservice.Call) (commandservice.Completion, error) {
			record := make([]byte, 64*1024)
			for {
				if _, err := call.Stdout.Write(record); err != nil {
					return commandservice.Completion{}, err
				}
				written.Add(1)
			}
		}})
		stream, err := server.Client.Invoke(testContext(t), invocation("write"))
		if err != nil {
			t.Fatal(err)
		}
		// Nothing reads the response, so every goroutine ends up waiting.
		synctest.Wait()
		if got := written.Load(); got != 5 {
			t.Errorf("the handler wrote %d records before it blocked, want 5: four queued and one sent", got)
		}
		stream.Cancel()
	})
}
