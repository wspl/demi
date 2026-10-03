package engine

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/runner/process"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	if handled, err := process.RunChildBootstrap(); handled {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	goleak.VerifyTestMain(m)
}

// shellFiles owns one script's files and reads them only after execution has joined.
func shellFiles(t *testing.T, root, script string, configure func(*Options)) (Result, string, string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	files := make([]*os.File, 3)
	for index := range files {
		file, err := os.CreateTemp(t.TempDir(), "shell")
		if err != nil {
			t.Fatal(err)
		}
		files[index] = file
		defer func() { _ = file.Close() }() // Cleanup also runs after cancellation closes the file.
	}
	options := Options{
		Cwd:    root,
		Env:    map[string]string{"HOME": root, "PATH": os.Getenv("PATH"), "TMPDIR": root},
		Stdin:  files[0],
		Stdout: files[1],
		Stderr: files[2],
	}
	if configure != nil {
		configure(&options)
	}
	result, err := Execute(ctx, script, options)
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := os.ReadFile(files[1].Name())
	if err != nil {
		t.Fatal(err)
	}
	stderr, err := os.ReadFile(files[2].Name())
	if err != nil {
		t.Fatal(err)
	}
	return result, string(stdout), string(stderr)
}

func TestShellHandlesRedirectsFunctionsSubshellCwdAndFreshState(t *testing.T) {
	root := t.TempDir()
	result, output, stderr := shellFiles(
		t,
		root,
		`mkdir a b; f() { printf '%s\n' "$1"; }; f pear > a/input; (cd a; cat input) | tr a-z A-Z; cd b; export LEAK=bad`,
		nil,
	)
	if result.Code != 0 || result.Cwd != filepath.Join(root, "b") || output != "PEAR\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
	result, output, stderr = shellFiles(t, result.Cwd, `printf '%s' "${LEAK-unset}"`, nil)
	if result.Code != 0 || output != "unset" {
		t.Fatalf("fresh state: result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestTeeAndOdUsePipelineStreams(t *testing.T) {
	if runtime.GOOS == "darwin" {
		t.Skip("decision 4: BSD od inserts extra spaces between hex bytes")
	}
	root := t.TempDir()
	result, output, stderr := shellFiles(t, root, `printf hello | tee made.txt | grep hello | od -An -tx1`, nil)
	data, err := os.ReadFile(filepath.Join(root, "made.txt"))
	if err != nil || string(data) != "hello" || result.Code != 0 || strings.TrimSpace(output) != "68 65 6c 6c 6f 0a" {
		t.Fatalf("file %q (%v), result %+v output %q stderr %q", data, err, result, output, stderr)
	}
}

func TestWcCountsAFileOfWholePagesWhole(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "pages.bin"), make([]byte, 5*65536), 0o600); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(t, root, "wc -c pages.bin", nil)
	if result.Code != 0 || strings.TrimSpace(output) != "327680 pages.bin" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestLoginProfilesApplyPerJobWithoutReplacingOwnedContextOrCwd(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "elsewhere"), 0o700); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{"first", "next"} {
		profile := "export FROM_PROFILE=" + value + "\nexport PATH=\"$HOME/tools\"\nexport DEMI_CONTEXT_ID=wrong\n" +
			"cd \"$HOME/elsewhere\"\n"
		if err := os.WriteFile(filepath.Join(root, ".bash_profile"), []byte(profile), 0o600); err != nil {
			t.Fatal(err)
		}
		result, output, stderr := shellFiles(
			t,
			root,
			`printf '%s\n' "$FROM_PROFILE" "$DEMI_CONTEXT_ID" "$PATH"`,
			func(options *Options) {
				options.Login = true
				options.Env["DEMI_CONTEXT_ID"] = "owned"
				options.Env["PATH"] = filepath.Join(root, "aliases")
			},
		)
		want := value + "\nowned\n" +
			filepath.Join(root, "aliases") + string(os.PathListSeparator) +
			filepath.Join(root, "tools") + "\n"
		if result.Code != 0 || result.Cwd != root || output != want {
			t.Fatalf("result %+v output %q want %q stderr %q", result, output, want, stderr)
		}
	}
}

func TestFgRefusesWithoutJobControl(t *testing.T) {
	result, output, stderr := shellFiles(t, t.TempDir(), `true & fg; echo "fg $?"`, nil)
	if result.Code != 0 || output != "fg 1\n" || !strings.Contains(stderr, "fg: no job control") {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

// These regression scripts run real child utilities, normally in under a second.
func TestRunnerShellRegressionStatusesAndRetainedInput(t *testing.T) {
	for _, scenario := range []struct {
		name, script, output string
		code                 uint8
	}{
		{"syntax", "cd sub; do", "", 2},
		{"pipeline", `false | true | false; printf '%s\n' "${PIPESTATUS[*]}"`, "1 0 1\n", 0},
		{"single", `false; printf '%s\n' "${PIPESTATUS[*]}"`, "1\n", 0},
		{"retained", `exec 9<&0; sh -c 'cat <&9; printf forwarded' <&9 & wait; printf done`, "inputforwardeddone", 0},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			root := t.TempDir()
			result, output, diagnostic := shellFiles(t, root, scenario.script, func(o *Options) {
				if _, err := o.Stdin.WriteString("input"); err != nil {
					t.Fatal(err)
				}
				if _, err := o.Stdin.Seek(0, 0); err != nil {
					t.Fatal(err)
				}
			})
			if result.Code != scenario.code || result.Cwd != root || output != scenario.output {
				t.Fatalf("result %+v output %q diagnostic %q", result, output, diagnostic)
			}
		})
	}
}

// Ubuntu's cloud-init profiles select local through a variable. These scripts
// exercise their assignment and bare-name forms without machine profile state.
func TestCloudInitDynamicLocalDeclarations(t *testing.T) {
	for _, scenario := range []struct {
		name, script, want string
	}{
		{"warnings", `warning=outside; f() {
command -v local >/dev/null && local _local="local" || typeset _local="typeset"
$_local warning="" idir="/var/lib/cloud/instance" n=0
$_local warndir="$idir/warnings"
$_local ufile="$HOME/.cloud-warnings.skip" sfile="$warndir/.skip"
printf '%s|%s|%s\n' "$warning" "$n" "$sfile"
}; f; printf '%s\n' "$warning"`, "|0|/var/lib/cloud/instance/warnings/.skip\noutside\n"},
		{"locale", `w1=outside; f() {
command -v local >/dev/null && local _local="local" || typeset _local="typeset"
$_local bad_names="" bad_lcs="" key="" val="" var="" vars="" bad_kv=""
$_local w1 w2 w3 w4 remain
read -r w1 w2 w3 w4 remain <<< 'LANG = en_US rest remaining words'
$_local bad invalid="" to_gen="" sfile="/usr/share/i18n/SUPPORTED"
$_local local pkgs=""
$_local pkgs=""
printf '%s|%s|%s|%s\n' "$bad_names" "$w1" "$remain" "$sfile"
}; f; printf '%s\n' "$w1"`, "|LANG|remaining words|/usr/share/i18n/SUPPORTED\noutside\n"},
		{"literal value", `f() { local cmd=local; "$cmd" 'value=$HOME $(echo wrong) *'; ` +
			`printf '%s\n' "$value"; }; f`, "$HOME $(echo wrong) *\n"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			result, output, stderr := shellFiles(t, t.TempDir(), scenario.script, nil)
			if result.Code != 0 || output != scenario.want || stderr != "" {
				t.Fatalf("result %+v output %q want %q stderr %q", result, output, scenario.want, stderr)
			}
		})
	}
}
