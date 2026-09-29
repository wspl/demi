package commandservice_test

import (
	"io"
	"net"
	"net/http"
	"testing"

	"github.com/wspl/demi/go/commandservice"
)

// earlyAnswer connects a client to a server of the wire that answers each
// request at once, before it has all of it, the way a service may: net/http
// then resets the request with NO_ERROR (RFC 9113 § 8.1).
func earlyAnswer(t *testing.T) *commandservice.Client {
	t.Helper()
	response := marshal(t,
		commandservice.Record{Kind: commandservice.RecordStdout, Data: []byte("{}")},
		commandservice.Record{Kind: commandservice.RecordCompletion},
	)
	answer := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		// A write fails only when the caller is gone, which the test shows.
		_, _ = w.Write(response)
	})
	var protocols http.Protocols
	protocols.SetUnencryptedHTTP2(true)
	server := &http.Server{
		Handler:   answer,
		Protocols: &protocols,
		// A window this small holds back most of what a caller sends.
		HTTP2: &http.HTTP2Config{MaxReceiveBufferPerStream: 16},
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	go server.Serve(listener)
	t.Cleanup(func() { server.Close() })
	conn, err := net.Dial("tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	client, err := commandservice.Connect(testContext(t), conn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	return client
}

// readRecords reads the records of a stream to its end.
func readRecords(t *testing.T, stream *commandservice.Stream) []commandservice.Record {
	t.Helper()
	var records []commandservice.Record
	for {
		record, err := stream.Next()
		if err == io.EOF {
			return records
		}
		if err != nil {
			t.Fatal(err)
		}
		records = append(records, record)
	}
}

// A service may answer before it has the whole request and then reset it with
// NO_ERROR: a conversation release it answered succeeds, and an input chunk
// written after its answer is dropped without an error.
func TestWhatACallerSendsAfterAnEarlyAnswerIsNotAFailure(t *testing.T) {
	client := earlyAnswer(t)
	// The first answer brings the service's settings: a caller may send
	// under the old window until it has seen them, so the requests that
	// follow are the ones its small window holds back.
	if err := post(t, client, http.MethodGet, commandservice.InfoPath, nil); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"first", "second"} {
		stream, err := client.Conversation(testContext(t), release(name))
		if err != nil {
			t.Fatal(err)
		}
		records := readRecords(t, stream)
		if len(records) != 2 || string(records[0].Data) != "{}" || records[1].Kind != commandservice.RecordCompletion {
			t.Errorf("release %s answered %+v", name, records)
		}
	}
	stream, err := client.Invoke(testContext(t), invocation("echo"))
	if err != nil {
		t.Fatal(err)
	}
	readRecords(t, stream)
	if err := stream.Write(make([]byte, 1024)); err != nil {
		t.Errorf("a chunk after the answer: %v", err)
	}
	if err := stream.End(); err != nil {
		t.Errorf("the end of input after the answer: %v", err)
	}
}
