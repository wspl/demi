package runners

import (
	"crypto/sha256"
	_ "embed"
	"fmt"
	"maps"
	"net/url"
	"slices"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/runnerwire"
	"mvdan.cc/sh/v3/syntax"
)

// BackendURL checks and returns the URL as an installer names it.
func BackendURL(value *url.URL) (*url.URL, error) {
	text := value.String()
	normalized, err := whatwg.Parse(text)
	if err != nil || (normalized.Scheme() != "http" && normalized.Scheme() != "https") || normalized.Username() != "" ||
		normalized.Password() != "" ||
		strings.Contains(normalized.Href(false), "?") ||
		strings.Contains(normalized.Href(false), "#") {
		return nil, fmt.Errorf("%s cannot be an installation's backend URL", text)
	}
	return url.Parse(normalized.Href(false))
}

// ShellScript renders the macOS and Linux shell installer for backend and release.
func ShellScript(backend *url.URL, release runnerwire.Release) string {
	cases := make([]string, 0, len(release.Targets))
	for _, target := range slices.Sorted(maps.Keys(release.Targets)) {
		cases = append(
			cases,
			fmt.Sprintf("\n  %s)\n    runner_hash=%s\n    ;;", target, shellWord(release.Targets[target].SHA256)),
		)
	}
	return strings.NewReplacer(
		"@BACKEND@", shellWord(backend.String()),
		"@RELEASE@", shellWord(release.Release),
		"@REGISTRATION@", shellWord(fmt.Sprintf("%x", sha256.Sum256([]byte(backend.String())))),
		"@BASE@", shellWord(backend.Scheme+"://"+backend.Host),
		"@CASES@", strings.Join(cases, "\n"),
	).Replace(shellTemplate)
}

// PowerShellScript renders the installer for the release's Windows targets.
func PowerShellScript(backend *url.URL, release runnerwire.Release) string {
	values := []string{}
	for _, target := range slices.Sorted(maps.Keys(release.Targets)) {
		if !strings.Contains(target, "windows") {
			continue
		}
		artifact := release.Targets[target]
		values = append(
			values,
			fmt.Sprintf(
				"  %s = @{ Hash = %s; Size = %d }",
				powershellWord(target),
				powershellWord(artifact.SHA256),
				artifact.Size,
			),
		)
	}
	return strings.NewReplacer(
		"@BACKEND@", powershellWord(backend.String()),
		"@RELEASE@", powershellWord(release.Release),
		"@REGISTRATION@", powershellWord(fmt.Sprintf("%x", sha256.Sum256([]byte(backend.String())))),
		"@BASE@", powershellWord(backend.Scheme+"://"+backend.Host),
		"@ARTIFACTS@", strings.Join(values, "\n"),
	).Replace(powershellTemplate)
}

//go:embed install.sh
var shellTemplate string

//go:embed install.ps1
var powershellTemplate string

// shellWord quotes validated installer values as POSIX shell words.
func shellWord(value string) string {
	// Installer inputs are normalized URLs and validated release identifiers:
	// none contains a NUL or an unprintable character that POSIX cannot quote.
	quoted, _ := syntax.Quote(value, syntax.LangPOSIX)
	return quoted
}

// powershellWord quotes an installer value as a PowerShell literal. No declared
// dependency implements PowerShell quoting; single quotes double inside literals.
func powershellWord(value string) string { return "'" + strings.ReplaceAll(value, "'", "''") + "'" }
