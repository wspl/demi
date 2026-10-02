//go:build darwin || linux

package shell

import (
	"context"
	"fmt"
	"math"
	"os"
	"os/exec"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runner/process"
	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

type resource struct {
	option             byte
	number             int
	description, label string
	scale              uint64
}

var resources = []resource{
	{'b', platformResource('b'), "socket buffer size", "bytes, ", 1},
	{'c', unix.RLIMIT_CORE, "core file size", "blocks, ", 512},
	{'d', unix.RLIMIT_DATA, "data seg size", "kbytes, ", 1024},
	{'e', platformResource('e'), "scheduling priority", "", 1},
	{'f', unix.RLIMIT_FSIZE, "file size", "blocks, ", 512},
	{'i', platformResource('i'), "pending signals", "", 1},
	{'k', -1, "max kqueues", "", 1},
	{'l', unix.RLIMIT_MEMLOCK, "max locked memory", "kbytes, ", 1024},
	{'m', unix.RLIMIT_RSS, "max memory size", "kbytes, ", 1024},
	{'n', unix.RLIMIT_NOFILE, "open files", "", 1},
	{'p', -2, "pipe size", "512 bytes, ", 512},
	{'q', platformResource('q'), "POSIX message queues", "bytes, ", 1},
	{'r', platformResource('r'), "real-time priority", "", 1},
	{'R', platformResource('R'), "real-time non-blocking time", "microseconds, ", 1},
	{'s', unix.RLIMIT_STACK, "stack size", "kbytes, ", 1024},
	{'t', unix.RLIMIT_CPU, "cpu time", "seconds, ", 1},
	{'u', unix.RLIMIT_NPROC, "max user processes", "", 1},
	{'v', unix.RLIMIT_AS, "virtual memory", "kbytes, ", 1024},
	{'x', platformResource('x'), "file locks", "", 1},
	{'P', -1, "number of pseudoterminals", "", 1},
	{'T', -1, "number of threads", "", 1},
}

type askedLimit struct {
	resource resource
	value    *string
}

func (e *execution) ulimit(ctx context.Context, args []string) error {
	soft, hard, all := false, false, false
	var asked []askedLimit
	for _, arg := range args[1:] {
		if strings.HasPrefix(arg, "-") && len(arg) > 1 {
			for _, option := range []byte(arg[1:]) {
				switch option {
				case 'S':
					soft = true
				case 'H':
					hard = true
				case 'a':
					all = true
				default:
					index := slices.IndexFunc(resources, func(r resource) bool { return r.option == option })
					if index < 0 {
						return diagnostic(ctx, 2, "ulimit: -%c: invalid option\n", option)
					}
					asked = append(asked, askedLimit{resource: resources[index]})
				}
			}
		} else {
			if len(asked) == 0 {
				asked = append(asked, askedLimit{resource: resources[4]})
			}
			if asked[len(asked)-1].value != nil {
				return diagnostic(ctx, 2, "ulimit: %s: too many arguments\n", arg)
			}
			value := arg
			asked[len(asked)-1].value = &value
		}
	}
	if all {
		asked = nil
		for _, r := range resources {
			asked = append(asked, askedLimit{resource: r})
		}
	} else if len(asked) == 0 {
		asked = append(asked, askedLimit{resource: resources[4]})
	}
	hc := interp.HandlerCtx(ctx)
	state := hc.Scope().(*interpreterScope)
	attributes := state.attributes
	attributes.Limits = slices.Clone(attributes.Limits)
	type shown struct {
		resource resource
		text     string
	}
	var showing []shown
	show := func(value uint64, r resource) string {
		if value == uint64(unix.RLIM_INFINITY) {
			return "unlimited"
		}
		return strconv.FormatUint(value/r.scale, 10)
	}
	for _, ask := range asked {
		r := ask.resource
		if r.number == -2 {
			if ask.value != nil {
				return diagnostic(ctx, 1, "ulimit: %s: cannot modify limit: Invalid argument\n", r.description)
			}
			showing = append(showing, shown{r, strconv.Itoa(pipeBuffer / 512)})
			continue
		}
		if r.number < 0 {
			if all {
				continue
			}
			return diagnostic(ctx, 1, "ulimit: -%c: not supported here\n", r.option)
		}
		low, high, err := process.ChildLimit(r.number)
		if err != nil {
			return err
		}
		for _, limit := range attributes.Limits {
			if limit.Resource == r.number {
				low, high = limit.Soft, limit.Hard
				break
			}
		}
		if ask.value == nil {
			value := low
			if hard {
				value = high
			}
			showing = append(showing, shown{r, show(value, r)})
			continue
		}
		value := uint64(0)
		switch *ask.value {
		case "unlimited":
			value = uint64(unix.RLIM_INFINITY)
		case "soft":
			value = low
		case "hard":
			value = high
		default:
			value, err = strconv.ParseUint(*ask.value, 10, 64)
			if err != nil || value > math.MaxUint64/r.scale {
				return diagnostic(ctx, 1, "ulimit: %s: invalid number\n", *ask.value)
			}
			value *= r.scale
		}
		both := soft == hard
		if (hard || both) && value > high && os.Geteuid() != 0 {
			return diagnostic(ctx, 1, "ulimit: %s: cannot modify limit: Operation not permitted\n", r.description)
		}
		if soft || both {
			low = value
		}
		if hard || both {
			high = value
		}
		attributes.Limits = slices.DeleteFunc(attributes.Limits, func(limit process.ResourceLimit) bool { return limit.Resource == r.number })
		attributes.Limits = append(attributes.Limits, process.ResourceLimit{Resource: r.number, Soft: low, Hard: high})
		command := process.Wrap(exec.Command("/bin/sh", "-c", ":"), false, attributes)
		if err := command.Start(ctx); err != nil {
			return diagnostic(ctx, 1, "ulimit: %s: cannot modify limit: %v\n", r.description, err)
		}
		status := command.Wait(ctx)
		if status.Error != nil {
			return diagnostic(ctx, 1, "ulimit: %s: cannot modify limit: %s\n", r.description, *status.Error)
		}
	}
	state.attributes = attributes
	for _, value := range showing {
		var err error
		if len(showing) == 1 {
			_, err = fmt.Fprintln(hc.Stdout, value.text)
		} else {
			_, err = fmt.Fprintf(hc.Stdout, "%-27s %18s %s\n", value.resource.description, fmt.Sprintf("(%s-%c)", value.resource.label, value.resource.option), value.text)
		}
		if err != nil {
			return err
		}
	}
	return nil
}
