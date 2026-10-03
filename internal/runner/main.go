package runner

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/host"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runner/shell"
	"github.com/wspl/demi/internal/runnerwire"
)

const program = "demi-runner"

// Main runs the runner or one declared-command alias. Bootstrap dispatch precedes
// flags, signal handling, service startup and process-limit inspection.
func Main(release string) int {
	if handled, err := process.RunChildBootstrap(); handled {
		return exitCode(0, err)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, terminationSignal())
	defer stop()
	name := strings.TrimSuffix(filepath.Base(os.Args[0]), filepath.Ext(os.Args[0]))
	var code uint8
	var err error
	if name != program {
		code, err = commandAlias(ctx, name, os.Args[1:])
	} else {
		code, err = runCLI(ctx, os.Args[1:], release)
	}
	return exitCode(code, err)
}

func exitCode(code uint8, err error) int {
	if err == nil {
		return int(code)
	}
	if errors.Is(err, syscall.EPIPE) {
		return 141
	}
	_, _ = fmt.Fprintln(os.Stderr, "demi-runner: "+err.Error()) // Exit status still reports a failed diagnostic write.
	if code != 0 {
		return int(code)
	}
	return 1
}

type cliOptions struct {
	action                                                          string
	backend                                                         *runnerwire.BackendURL
	home, release, boot, name, managed, artifacts                   string
	managedSet, homeSet, bootSet, nameSet, artifactsSet, releaseSet bool
}

func parseCLI(args []string, release string) (cliOptions, error) {
	options := cliOptions{release: release}
	if len(args) == 0 {
		return options, errors.New("a subcommand is required: run, status, drain")
	}
	options.action = args[0]
	if options.action != "run" && options.action != "status" && options.action != "drain" {
		return options, fmt.Errorf("unrecognized subcommand '%s'", options.action)
	}
	flags := flag.NewFlagSet(program+" "+options.action, flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	backend := flags.String("backend", "", "The backend the installation belongs to.")
	flags.StringVar(
		&options.home,
		"home",
		os.Getenv("DEMI_HOME"),
		"The installation's directory; one per backend under ~/.demi/instances by default.",
	)
	selected, releaseSet := os.LookupEnv("DEMI_RELEASE_ID")
	options.releaseSet = releaseSet
	_, options.homeSet = os.LookupEnv("DEMI_HOME")
	if !releaseSet {
		selected = release
	}
	flags.StringVar(
		&options.release,
		"release",
		selected,
		"This runner's release, which status compares with the active one.",
	)
	if options.action == "run" {
		options.runFlags(flags)
	}
	if err := flags.Parse(args[1:]); err != nil {
		return options, err
	}
	if flags.NArg() != 0 {
		return options, fmt.Errorf("unexpected argument '%s'", flags.Arg(0))
	}
	backendSet := options.selectedFlags(flags)
	if backendSet {
		parsed, err := runnerwire.ParseBackendURL(*backend)
		if err != nil {
			return options, err
		}
		options.backend = &parsed
	}
	if options.bootSet && options.backend != nil {
		return options, errors.New("--managed-boot cannot be used with --backend")
	}
	return options, nil
}

func runCLI(ctx context.Context, args []string, release string) (uint8, error) {
	if len(args) > 0 {
		switch args[0] {
		case "--help", "-h":
			return 0, printHelp("")
		case "--version", "-V":
			_, err := fmt.Fprintln(os.Stdout, program+" "+release)
			return 0, err
		case "help":
			if len(args) == 1 {
				return 0, printHelp("")
			}
			args = []string{args[1], "--help"}
		}
	}
	options, err := parseCLI(args, release)
	if errors.Is(err, flag.ErrHelp) {
		return 0, printHelp(options.action)
	}
	if err != nil {
		return 2, err
	}
	boot, err := readManagedBoot(options)
	if err != nil {
		return 0, err
	}
	directory, err := installationDirectory(options, boot)
	if err != nil {
		return 0, err
	}
	if err := openInstallation(ctx, directory); err != nil {
		return 0, err
	}
	state := runnerState{root: directory}
	if options.action != "run" {
		var wanted *string
		// Only an explicitly selected release changes status's exit status.
		if options.releaseSet {
			wanted = &options.release
		}
		return manage(ctx, state, action(options.action), wanted)
	}
	backend, err := selectedBackend(options, boot, state)
	if err != nil {
		return 0, err
	}
	home, info, err := runnerInfo(options, boot)
	if err != nil {
		return 0, err
	}
	return startRunner(ctx, options, boot, directory, backend, home, info)
}

func installationDirectory(options cliOptions, boot *runnerwire.ManagedBoot) (string, error) {
	if boot != nil {
		return "/run/demi", nil
	}
	if options.homeSet {
		return options.home, nil
	}
	if options.backend == nil {
		return "", errors.New("pass --backend <url> to select an installation")
	}
	home, err := userHome()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".demi/instances", instanceID(*options.backend)), nil
}

func userHome() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return "", errors.New("user home is not configured")
	}
	return home, nil
}

func manage(ctx context.Context, state runnerState, action action, release *string) (uint8, error) {
	active, err := state.active()
	if err != nil {
		return 0, err
	}
	args, err := (managementRequest{Secret: active.Secret, Action: action}).MarshalJSON()
	if err != nil {
		return 0, err
	}
	cwd, err := os.Getwd()
	if err != nil {
		return 0, err
	}
	id, err := uuid.NewRandom()
	if err != nil {
		return 0, err
	}
	stdout, err := process.StandardFile(ctx, 1)
	if err != nil {
		return 0, err
	}
	defer func() { _ = stdout.Close() }() // Forward owns it; this covers setup failure.
	stderr, err := process.StandardFile(ctx, 2)
	if err != nil {
		return 0, err
	}
	defer func() { _ = stderr.Close() }()
	completion, err := process.Forward(
		ctx,
		active.Endpoint,
		commandwire.LocalInvocation{
			Operation:    manageOperation,
			InvocationID: strings.ReplaceAll(id.String(), "-", ""),
			Args:         args,
			Cwd:          cwd,
			Env:          map[string]string{},
		},
		process.Stdio{Stdin: io.NopCloser(strings.NewReader("")), Stdout: stdout, Stderr: stderr},
	)
	if err != nil || completion.ExitCode != 0 {
		return completion.ExitCode, err
	}
	if action == drainAction {
		return waitInstallation(ctx, state.root)
	}
	if release != nil && *release != active.Release {
		return 3, nil
	}
	return 0, nil
}

const runnerHelp = `Runs this device as a Demi execution target.

Usage: demi-runner <COMMAND>

Commands:
  run     Connects to the backend and serves its work.
  status  Reports whether the installation's runner is active; exits with 3 when it runs another release.
  drain   Stops admitting work, waits for the running jobs, and releases the installation.
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
`

// printHelp prints the selected runner command's description, usage line and public options.
func printHelp(action string) error {
	if action == "" {
		_, err := fmt.Fprint(os.Stdout, runnerHelp)
		return err
	}
	description := ""
	switch action {
	case "run":
		description = "Connects to the backend and serves its work."
	case "status":
		description = "Reports whether the installation's runner is active; exits with 3 when it runs another release."
	case "drain":
		description = "Stops admitting work, waits for the running jobs, and releases the installation."
	}
	options := "  --backend <BACKEND>  The backend the installation belongs to.\n"
	if action == "run" {
		options += "  --managed-boot <MANAGED_BOOT>  A managed guest's boot record, which names the backend.\n"
	}
	_, err := fmt.Fprintf(
		os.Stdout,
		"%s\n\nUsage: demi-runner %s [OPTIONS]\n\nOptions:\n%s  -h, --help  Print help\n",
		description,
		action,
		options,
	)
	return err
}

func (o *cliOptions) runFlags(flags *flag.FlagSet) {
	_, o.nameSet = os.LookupEnv("DEMI_RUNNER_NAME")
	_, o.artifactsSet = os.LookupEnv("DEMI_ARTIFACTS")
	flags.StringVar(&o.boot, "managed-boot", "", "A managed guest's boot record, which names the backend.")
	flags.StringVar(
		&o.name,
		"name",
		os.Getenv("DEMI_RUNNER_NAME"),
		"How the device appears to the backend; the hostname by default.",
	)
	o.managed, o.managedSet = os.LookupEnv("DEMI_RUNNER_MANAGED")
	flags.StringVar(
		&o.managed,
		"managed",
		o.managed,
		"Marks a runner the backend manages, when set and not empty.",
	)
	flags.StringVar(
		&o.artifacts,
		"artifacts",
		os.Getenv("DEMI_ARTIFACTS"),
		"The artifact cache, which runners of one user may share; artifacts in the installation's directory by default.",
	)
}

func (o *cliOptions) selectedFlags(flags *flag.FlagSet) (backendSet bool) {
	flags.Visit(func(f *flag.Flag) {
		switch f.Name {
		case "managed":
			o.managedSet = true
		case "home":
			o.homeSet = true
		case "managed-boot":
			o.bootSet = true
		case "name":
			o.nameSet = true
		case "artifacts":
			o.artifactsSet = true
		case "release":
			o.releaseSet = true
		case "backend":
			backendSet = true
		}
	})
	return backendSet
}

func readManagedBoot(options cliOptions) (*runnerwire.ManagedBoot, error) {
	var boot *runnerwire.ManagedBoot
	if options.bootSet {
		data, err := os.ReadFile(options.boot)
		if err != nil {
			return nil, err
		}
		record, err := runnerwire.DecodeManagedBoot(data)
		if err != nil {
			return nil, err
		}
		boot = &record
	}
	return boot, nil
}

func selectedBackend(
	options cliOptions,
	boot *runnerwire.ManagedBoot,
	state runnerState,
) (*runnerwire.BackendURL, error) {
	backend := options.backend
	if boot != nil {
		backend = &boot.BackendURL
	}
	if backend == nil {
		config, found, err := state.config()
		if err != nil {
			return nil, err
		}
		if !found {
			return nil, errors.New("pass --backend <url> on first start")
		}
		backend = &config.BackendURL
	}
	return backend, nil
}

func runnerInfo(options cliOptions, boot *runnerwire.ManagedBoot) (string, runnerwire.RunnerInfo, error) {
	home, err := userHome()
	if err != nil {
		return "", runnerwire.RunnerInfo{}, err
	}
	hostname, err := os.Hostname()
	if err != nil {
		return "", runnerwire.RunnerInfo{}, err
	}
	if !utf8.ValidString(hostname) {
		return "", runnerwire.RunnerInfo{}, errors.New("the hostname is not UTF-8")
	}
	identity := runnerwire.HostIdentity{Hostname: hostname, HomeDir: home}
	if runtime.GOOS != "windows" {
		identity.UID = uint32(os.Getuid())
		identity.GID = uint32(os.Getgid())
	}
	name := options.name
	if !options.nameSet {
		name = hostname
	}
	platform := runnerwire.RunnerPlatformLinux
	if runtime.GOOS == "darwin" {
		platform = runnerwire.RunnerPlatformDarwin
	}
	if runtime.GOOS == "windows" {
		platform = runnerwire.RunnerPlatformWin32
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		return "", runnerwire.RunnerInfo{}, err
	}
	targetName := string(target)
	info := runnerwire.RunnerInfo{
		Name:         name,
		Platform:     platform,
		Version:      options.release,
		NativeTarget: &targetName,
		Identity:     identity,
	}
	if boot != nil || options.managedSet {
		managed := boot != nil || options.managed != ""
		info.Managed = &managed
	}
	return home, info, nil
}

func startRunner(
	ctx context.Context,
	options cliOptions,
	boot *runnerwire.ManagedBoot,
	directory string,
	backend *runnerwire.BackendURL,
	home string,
	info runnerwire.RunnerInfo,
) (uint8, error) {
	logDirectory, jobRoot, artifacts, volumes, token := startupPaths(options, boot, directory, home)
	log, err := openHostLog(ctx, logDirectory)
	if err != nil {
		return 0, err
	}
	previous := slog.Default()
	slog.SetDefault(slog.New(&logHandler{log: log}))
	defer func() {
		slog.SetDefault(previous)
		_ = log.close(context.Background()) // Background cannot cancel the writer join.
	}()
	startupLimits()
	executable, err := os.Executable()
	if err != nil {
		return 0, err
	}
	cwd, err := os.Getwd()
	if err != nil {
		return 0, err
	}
	err = runRegistration(
		ctx,
		registrationOptions{
			backend:    *backend,
			directory:  directory,
			artifacts:  artifacts,
			jobRoot:    jobRoot,
			executable: executable,
			cwd:        cwd,
			env:        processEnvironment(),
			runner:     info,
			token:      token,
			volumes:    volumes,
			shell:      &shell.Shell{},
			log:        log,
		},
	)
	if err != nil {
		slog.Error(err.Error())
		return 1, nil
	}
	return 0, nil
}

func waitInstallation(ctx context.Context, root string) (uint8, error) {
	for {
		lease, err := tryInstallationLock(root)
		if err == nil {
			return 0, lease.close()
		}
		if !errors.Is(err, errInstallationBusy) {
			return 0, err
		}
		timer := time.NewTimer(50 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return 0, ctx.Err()
		case <-timer.C:
		}
	}
}

func startupPaths(
	options cliOptions,
	boot *runnerwire.ManagedBoot,
	directory, home string,
) (logDirectory, jobRoot, artifacts string, volumes []host.ManagedVolume, token *runnerwire.DeviceToken) {
	logDirectory = filepath.Join(directory, "log")
	jobRoot = filepath.Join(directory, "jobs")
	artifacts = options.artifacts
	if !options.artifactsSet {
		artifacts = filepath.Join(directory, "artifacts")
	}
	if boot != nil {
		logDirectory = "/var/log/demi"
		jobRoot = "/var/lib/demi/jobs"
		if !options.artifactsSet {
			artifacts = filepath.Join(home, ".demi/artifacts")
		}
		token = &boot.DeviceToken
		volumes = []host.ManagedVolume{
			{Name: runnerwire.VolumeNameSystem, Mount: "/"},
			{Name: runnerwire.VolumeNameHome, Mount: "/home"},
		}
	}
	return
}
