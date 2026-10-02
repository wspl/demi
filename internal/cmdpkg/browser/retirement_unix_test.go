//go:build darwin || linux

package browser

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandwire"
)

// A fixture process writes Chrome's refusal and exits at once. Repeated launches
// protect the stderr/exit race; no real Chrome or wall-time wait is needed.
func TestLaunchAsRootSaysToRunRunnerAsOrdinaryUser(t *testing.T) {
	launcher := filepath.Join(t.TempDir(), "root-chrome")
	script := "#!/bin/sh\necho '[1:1:0927/010848.716678:ERROR:content/browser/zygote_host/zygote_host_impl_linux.cc:102] Running as root without --no-sandbox is not supported. See https://crbug.com/638180.' >&2\nexit 1\n"
	if err := os.WriteFile(launcher, []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	for range 8 {
		environment, err := tabs.Launch(t.Context(), tabs.LaunchOptions{Executable: launcher, Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}}, &tabstest.Numbers{})
		if environment != nil {
			if cleanup := environment.Close(t.Context()); cleanup != nil {
				t.Error(cleanup)
			}
			t.Fatal("refused launch returned a browser")
		}
		var failure *cdp.BrowserError
		if !errors.As(err, &failure) || failure.Kind != cdp.KindRoot || !strings.Contains(err.Error(), "run the runner as an ordinary user") {
			t.Fatalf("root refusal: %v", err)
		}
	}
}
