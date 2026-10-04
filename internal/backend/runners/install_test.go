package runners_test

import (
	"crypto/sha256"
	"fmt"
	"net/url"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/runnerproto"
)

func TestInstallerBackendURL(t *testing.T) {
	for _, text := range []string{"http://localhost:3271/", "https://demi.example.com/base"} {
		value, err := url.Parse(text)
		if err != nil {
			t.Fatal(err)
		}
		if _, err = runners.BackendURL(value); err != nil {
			t.Fatalf("usable %s: %v", text, err)
		}
	}
	for _, text := range []string{
		"ftp://demi.example.com/",
		"https://user@demi.example.com/",
		"https://user:secret@demi.example.com/",
		"https://demi.example.com/?token=1",
		"https://demi.example.com/#fragment",
		"https://demi.example.com/?",
	} {
		value, err := url.Parse(text)
		if err != nil {
			t.Fatal(err)
		}
		if _, err = runners.BackendURL(value); err == nil {
			t.Fatalf("accepted %s", text)
		}
	}
}

func TestInstallersNameReleaseAndBackend(t *testing.T) {
	value, err := url.Parse("https://DEMI.example.com:443/base")
	if err != nil {
		t.Fatal(err)
	}
	backend, err := runners.BackendURL(value)
	if err != nil {
		t.Fatal(err)
	}
	release := runnerproto.Release{
		Release: strings.Repeat("a", 64),
		Targets: map[string]cmdproto.PackageArtifact{
			"aarch64-apple-darwin":   {SHA256: strings.Repeat("b", 64), Size: 42},
			"x86_64-pc-windows-msvc": {SHA256: strings.Repeat("c", 64), Size: 43},
		},
	}
	for _, script := range []string{runners.ShellScript(backend, release), runners.PowerShellScript(backend, release)} {
		for _, part := range []string{
			backend.String(),
			release.Release,
			fmt.Sprintf("%x", sha256.Sum256([]byte(backend.String()))),
			strings.Repeat("c", 64),
		} {
			if !strings.Contains(script, part) {
				t.Fatalf("script lacks %s", part)
			}
		}
		if strings.Contains(script, "@BACKEND@") || strings.Contains(script, "@CASES@") ||
			strings.Contains(script, "@ARTIFACTS@") {
			t.Fatal("template not rendered")
		}
	}
	shell := runners.ShellScript(backend, release)
	if !strings.Contains(shell, strings.Repeat("b", 64)) {
		t.Fatal("shell lacks Mac target")
	}
	powershell := runners.PowerShellScript(backend, release)
	if strings.Contains(powershell, strings.Repeat("b", 64)) {
		t.Fatal("PowerShell offered non-Windows target")
	}
}
