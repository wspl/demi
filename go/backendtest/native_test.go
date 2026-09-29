package backendtest_test

import (
	"bytes"
	"net/http"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// The harness publishes the native packages the workspace built as development
// releases, which the backend's local store serves itself, so every scenario
// whose runner runs a native command, on a paired device or on the Cloud,
// installs it from the backend. This one shows the store's own promises: a
// runner installs a package from the backend's route and runs it, and the route
// serves only what the backend loaded.
//
// Cost: one backend and a real runner, over a second: the runner installs the
// native fixture package from the backend.
func TestARunnerInstallsADevelopmentReleaseFromTheBackendWhichServesNothingElse(t *testing.T) {
	t.Parallel()
	b, master, _ := deviceConversation(t, backendtest.New(t, backendtest.WithNativeFixture()))
	// The stream runs the fixture package's echo, whose program the laptop's
	// runner downloaded from the backend, over plain HTTP.
	echo := b.OpenStream(master, streamConversation, "echo")
	echo.Answered()
	echo.Close()

	// The route serves the loaded executable whole, as an immutable file.
	digest, size, program := backendtest.FixturePackage.Artifact(t)
	served := b.Get("/native-artifacts/"+digest, nil).Expect(http.StatusOK)
	want, err := os.ReadFile(program)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(served.Body, want) {
		t.Fatal("the served bytes differ from the program")
	}
	if got := served.Header.Get("Content-Length"); got != strconv.FormatUint(size, 10) {
		t.Fatalf("Content-Length is %s, not %d", got, size)
	}
	headerIs(t, served, "Cache-Control", "public, max-age=31536000, immutable")
	// A digest no loaded release carries, or no digest at all, is not there.
	for _, unknown := range []string{strings.Repeat("0", 64), "demi-native-fixture"} {
		wantRefusal(t, b.Get("/native-artifacts/"+unknown, nil), http.StatusNotFound, "not_found", "the artifact "+unknown)
	}
	b.Stop()
}
