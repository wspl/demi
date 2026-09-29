package claude_test

import (
	"os"
	"testing"

	"github.com/wspl/demi/go/claude"
)

// asProgram makes the test binary the demi-claude program when it is started
// with it set, so a test starts the program as a runner does.
const asProgram = "DEMI_CLAUDE_TEST_AS_PROGRAM"

func TestMain(m *testing.M) {
	if os.Getenv(asProgram) != "" {
		claude.Main()
	}
	os.Exit(m.Run())
}
