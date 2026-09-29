package shell

import "golang.org/x/sys/unix"

var resourceNumbers = map[rune]int{
	'c': unix.RLIMIT_CORE, 'd': unix.RLIMIT_DATA, 'f': unix.RLIMIT_FSIZE,
	'l': unix.RLIMIT_MEMLOCK, 'm': unix.RLIMIT_RSS, 'n': unix.RLIMIT_NOFILE,
	's': unix.RLIMIT_STACK, 't': unix.RLIMIT_CPU, 'u': unix.RLIMIT_NPROC, 'v': unix.RLIMIT_AS,
}

const pipeBuffer = 512
