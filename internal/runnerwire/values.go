package runnerwire

// A signal a kill request names (`runner.md` § Host operations): one
// closed set that raw processes and jobs share.
// +demi:root
// +demi:enum SIGTERM SIGKILL SIGINT SIGHUP SIGQUIT SIGUSR1 SIGUSR2 SIGSTOP SIGCONT
type Signal string

// Signals accepted by raw-process and job kill requests.
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
// `process.platform` names it. A device keeps its runner's, and the web app
// receives it with the device.
// +demi:root
// +demi:enum darwin win32 linux
type RunnerPlatform string

// Platforms use the names reported by Node process.platform.
const (
	RunnerPlatformDarwin RunnerPlatform = "darwin"
	RunnerPlatformWin32  RunnerPlatform = "win32"
	RunnerPlatformLinux  RunnerPlatform = "linux"
)
