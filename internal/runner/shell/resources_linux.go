package shell

import "golang.org/x/sys/unix"

const pipeBuffer = 4096

func platformResource(option byte) int {
	switch option {
	case 'e':
		return unix.RLIMIT_NICE
	case 'i':
		return unix.RLIMIT_SIGPENDING
	case 'q':
		return unix.RLIMIT_MSGQUEUE
	case 'r':
		return unix.RLIMIT_RTPRIO
	case 'R':
		return unix.RLIMIT_RTTIME
	case 'x':
		return unix.RLIMIT_LOCKS
	default:
		return -1
	}
}
