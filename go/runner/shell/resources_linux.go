package shell

import "golang.org/x/sys/unix"

var resourceNumbers = map[rune]int{
	'c': unix.RLIMIT_CORE, 'd': unix.RLIMIT_DATA, 'e': unix.RLIMIT_NICE,
	'f': unix.RLIMIT_FSIZE, 'i': unix.RLIMIT_SIGPENDING, 'l': unix.RLIMIT_MEMLOCK,
	'm': unix.RLIMIT_RSS, 'n': unix.RLIMIT_NOFILE, 'q': unix.RLIMIT_MSGQUEUE,
	'r': unix.RLIMIT_RTPRIO, 'R': unix.RLIMIT_RTTIME, 's': unix.RLIMIT_STACK,
	't': unix.RLIMIT_CPU, 'u': unix.RLIMIT_NPROC, 'v': unix.RLIMIT_AS, 'x': unix.RLIMIT_LOCKS,
}

const pipeBuffer = 4096
