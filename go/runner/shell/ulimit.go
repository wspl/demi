package shell

import (
	"context"
	"fmt"
	"math"
	"strconv"
	"strings"

	"mvdan.cc/sh/v3/interp"
)

type limitResource struct {
	option            rune
	description, unit string
	scale             uint64
}

var limitResources = []limitResource{
	{'b', "socket buffer size", "bytes, ", 1},
	{'c', "core file size", "blocks, ", 512},
	{'d', "data seg size", "kbytes, ", 1024},
	{'e', "scheduling priority", "", 1},
	{'f', "file size", "blocks, ", 512},
	{'i', "pending signals", "", 1},
	{'k', "max kqueues", "", 1},
	{'l', "max locked memory", "kbytes, ", 1024},
	{'m', "max memory size", "kbytes, ", 1024},
	{'n', "open files", "", 1},
	{'p', "pipe size", "512 bytes, ", 512},
	{'q', "POSIX message queues", "bytes, ", 1},
	{'r', "real-time priority", "", 1},
	{'R', "real-time non-blocking time", "microseconds, ", 1},
	{'s', "stack size", "kbytes, ", 1024},
	{'t', "cpu time", "seconds, ", 1},
	{'u', "max user processes", "", 1},
	{'v', "virtual memory", "kbytes, ", 1024},
	{'x', "file locks", "", 1},
	{'P', "number of pseudoterminals", "", 1},
	{'T', "number of threads", "", 1},
}

type askedLimit struct {
	resource limitResource
	value    *string
}

func ulimit(ctx context.Context, args []string) error {
	var soft, hard, all bool
	var asked []askedLimit
	for _, arg := range args[1:] {
		if !strings.HasPrefix(arg, "-") || arg == "-" {
			if len(asked) == 0 {
				asked = append(asked, askedLimit{resource: limitResources[4]})
			}
			last := &asked[len(asked)-1]
			if last.value != nil {
				return usage(ctx, "ulimit", arg+": too many arguments")
			}
			value := arg
			last.value = &value
			continue
		}
		for _, option := range arg[1:] {
			switch option {
			case 'S':
				soft = true
			case 'H':
				hard = true
			case 'a':
				all = true
			default:
				found := false
				for _, resource := range limitResources {
					if option == resource.option {
						asked = append(asked, askedLimit{resource: resource})
						found = true
						break
					}
				}
				if !found {
					return usage(ctx, "ulimit", fmt.Sprintf("-%c: invalid option", option))
				}
			}
		}
	}
	if all {
		asked = nil
		for _, resource := range limitResources {
			asked = append(asked, askedLimit{resource: resource})
		}
	} else if len(asked) == 0 {
		asked = append(asked, askedLimit{resource: limitResources[4]})
	}
	setup, err := childAttributes(ctx)
	if err != nil {
		return err
	}
	type showing struct {
		resource limitResource
		value    string
	}
	var output []showing
	for _, ask := range asked {
		res := ask.resource
		if res.option == 'p' {
			if ask.value != nil {
				return diagnostic(ctx, "ulimit", fmt.Errorf("pipe size: cannot modify limit: Invalid argument"))
			}
			output = append(output, showing{res, strconv.Itoa(pipeBuffer / 512)})
			continue
		}
		resource, ok := resourceNumbers[res.option]
		if !ok {
			if all {
				continue
			}
			return diagnostic(ctx, "ulimit", fmt.Errorf("-%c: not supported here", res.option))
		}
		lo, hi, err := resourceLimit(ctx, resource)
		if err != nil {
			return err
		}
		index := -1
		for i, limit := range setup.Limits {
			if limit.Resource == resource {
				lo, hi, index = limit.Soft, limit.Hard, i
				break
			}
		}
		if ask.value == nil {
			value := lo
			if hard {
				value = hi
			}
			text := "unlimited"
			if value != infinity() {
				text = strconv.FormatUint(value/res.scale, 10)
			}
			output = append(output, showing{res, text})
			continue
		}
		var value uint64
		switch *ask.value {
		case "unlimited":
			value = infinity()
		case "hard":
			value = hi
		case "soft":
			value = lo
		default:
			parsed, err := strconv.ParseUint(strings.TrimPrefix(*ask.value, "+"), 10, 64)
			if err != nil || parsed > math.MaxUint64/res.scale {
				return diagnostic(ctx, "ulimit", fmt.Errorf("%s: invalid number", *ask.value))
			}
			value = parsed * res.scale
		}
		if soft || soft == hard {
			lo = value
		}
		if hard || soft == hard {
			hi = value
		}
		limit := childLimit{resource, lo, hi}
		if index < 0 {
			setup.Limits = append(setup.Limits, limit)
		} else {
			setup.Limits[index] = limit
		}
		if _, err := probeAttributes(ctx, setup); err != nil {
			return diagnostic(ctx, "ulimit", fmt.Errorf("%s: cannot modify limit: %w", res.description, err))
		}
	}
	// Commit only after all proposed changes have passed native validation.
	for _, limit := range setup.Limits {
		for _, resource := range limitResources {
			number, ok := resourceNumbers[resource.option]
			if !ok || number != limit.Resource {
				continue
			}
			if err := setAttribute(ctx, limitVariable("S", resource.option), strconv.FormatUint(limit.Soft, 10)); err != nil {
				return err
			}
			if err := setAttribute(ctx, limitVariable("H", resource.option), strconv.FormatUint(limit.Hard, 10)); err != nil {
				return err
			}
		}
	}
	for _, shown := range output {
		if len(output) == 1 {
			_, err = fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, shown.value)
		} else {
			label := fmt.Sprintf("(%s-%c)", shown.resource.unit, shown.resource.option)
			_, err = fmt.Fprintf(interp.HandlerCtx(ctx).Stdout, "%-27s %18s %s\n", shown.resource.description, label, shown.value)
		}
		if err != nil {
			return err
		}
	}
	return nil
}
