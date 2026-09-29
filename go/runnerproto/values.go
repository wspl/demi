package runnerproto

// Signal is a signal accepted by raw-process and job kill requests.
//
//demi:enum
type Signal string

const (
	SignalTerminate Signal = "SIGTERM"
	SignalKill      Signal = "SIGKILL"
	SignalInterrupt Signal = "SIGINT"
	SignalHangup    Signal = "SIGHUP"
	SignalQuit      Signal = "SIGQUIT"
	SignalUser1     Signal = "SIGUSR1"
	SignalUser2     Signal = "SIGUSR2"
	SignalStop      Signal = "SIGSTOP"
	SignalContinue  Signal = "SIGCONT"
)

// The operating system a runner runs on, named as Node's
// `process.platform` names it. A device keeps its runner's, and the browser
// receives it with the device.
//
//demi:enum
type RunnerPlatform string

const (
	RunnerPlatformDarwin RunnerPlatform = "darwin"
	RunnerPlatformWin32  RunnerPlatform = "win32"
	RunnerPlatformLinux  RunnerPlatform = "linux"
)

// ParseRunnerPlatform validates a platform stored by its wire name.
func ParseRunnerPlatform(name string) (RunnerPlatform, error) {
	switch RunnerPlatform(name) {
	case RunnerPlatformDarwin, RunnerPlatformWin32, RunnerPlatformLinux:
		return RunnerPlatform(name), nil
	}
	return "", &InvalidError{Rule: "unknown runner platform"}
}
