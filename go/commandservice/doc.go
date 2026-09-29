// Package commandservice is the command wire between a runner and a command
// program, and the SDK that speaks both ends of it. It is the Go counterpart of
// the Rust crate command-service; docs/execution/native-runtime.md defines the
// behavior.
//
// The wire is HTTP/2 without TLS over a connection the caller supplies, usually
// the standard input and output of a child process. The runner is the client
// and the command program is the server:
//
//	GET  /v1/info          the protocol version and the operations served
//	POST /v1/invoke        one invocation: metadata, input chunks, records
//	POST /v1/conversation  release a conversation's state, or list what is held
//	POST /v1/numbers       the service's requests for conversation numbers
//	POST /v1/shutdown      stop admission and drain
//
// A command program implements [Handler] and passes it to [ServeStdio]. A
// caller connects with [Connect], drives one invocation with [Client.Invoke]
// and [Exchange], and answers a service's numbers stream with
// [NumbersStream.Answer].
//
// Every value that enters from the other side of the wire is checked where it
// enters, against a schema derived from its Go type ([Decode]), and every value
// that leaves is checked before it is sent ([Encode]).
package commandservice
