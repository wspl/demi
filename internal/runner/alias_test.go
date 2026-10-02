package runner

import (
	"testing"

	"github.com/wspl/demi/internal/runner/process"
)

func TestCommandAliasRequiresExecutionAuthority(t *testing.T) {
	t.Setenv(process.EndpointEnv, "unused")
	t.Setenv(process.ContextEnv, "invalid")
	_, err := commandAlias(t.Context(), "fixture", nil)
	if err == nil {
		t.Fatal("invalid execution authority accepted")
	}
}
