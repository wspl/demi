package commandservice_test

import (
	"encoding/json/jsontext"
	"fmt"
	"testing"

	"github.com/wspl/demi/go/commandservice"
)

// typicalInvocation is what a runner sends for a command: a short argument
// object and the environment of a shell.
func typicalInvocation(b testing.TB) []byte {
	b.Helper()
	call := invocation("read")
	call.Args = jsontext.Value(`{"path":"src/main.go","offset":0,"limit":200}`)
	call.Cwd = "/home/user/project"
	call.Env = map[string]string{}
	for i := range 20 {
		call.Env[fmt.Sprintf("DEMI_VARIABLE_%02d", i)] = "/usr/local/bin:/usr/bin:/bin"
	}
	data, err := commandservice.Encode(call)
	if err != nil {
		b.Fatal(err)
	}
	return data
}

func BenchmarkDecodeInvocation(b *testing.B) {
	data := typicalInvocation(b)
	b.ReportAllocs()
	for b.Loop() {
		if _, err := commandservice.Decode[commandservice.Invocation](data); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkEncodeInvocation(b *testing.B) {
	data := typicalInvocation(b)
	call, err := commandservice.Decode[commandservice.Invocation](data)
	if err != nil {
		b.Fatal(err)
	}
	b.ReportAllocs()
	for b.Loop() {
		if _, err := commandservice.Encode(call); err != nil {
			b.Fatal(err)
		}
	}
}
