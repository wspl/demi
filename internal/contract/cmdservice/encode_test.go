package cmdservice

import (
	"testing"

	"github.com/wspl/demi/internal/contract/zodrt"
)

func validContext() CommandContext {
	return CommandContext{
		Conversation: "c1",
		Caller:       AgentCaller{Node: "n1"},
		Locale:       CommandLocale{TimeZone: "Europe/Berlin", Languages: []string{"de-DE"}},
	}
}

func TestEncodersRefuseInvalidValues(t *testing.T) {
	descriptor := PackageDescriptor{
		ID:         "demi.commands",
		Version:    "1.0.0",
		Operations: []string{"read", "read"},
	}
	if _, err := EncodePackageDescriptorJSON(descriptor); err == nil {
		t.Error("repeated operations encoded")
	}
	descriptor.Operations = []string{"read"}
	if _, err := EncodePackageDescriptorJSON(descriptor); err != nil {
		t.Errorf("a valid descriptor: %v", err)
	}
	var env zodrt.Record[string, string]
	env.Set("A=B", "c")
	invocation := Invocation{
		Operation:    "read",
		InvocationID: "i1",
		Context:      validContext(),
		Cwd:          "/work",
		Env:          env,
	}
	if _, err := EncodeInvocationJSON(invocation); err == nil {
		t.Error("an environment name with = encoded")
	}
	invocation.Env = zodrt.Record[string, string]{}
	invocation.Env.Set("A", "c")
	invocation.Context.Caller = nil
	if _, err := EncodeInvocationJSON(invocation); err == nil {
		t.Error("a context without a caller encoded")
	}
	if _, err := EncodeCompletionJSON(Completion{ExitCode: 256}); err == nil {
		t.Error("an exit code above 255 encoded")
	}
	if _, err := EncodeArtifactLocationJSON(ArtifactURL{URL: "not a url"}); err == nil {
		t.Error("an artifact location without a URL encoded")
	}
	if _, err := EncodeCommandLocaleJSON(CommandLocale{TimeZone: "UTC", Languages: make([]string, CommandLocaleLanguages+1)}); err == nil {
		t.Error("too many languages encoded")
	}
}
