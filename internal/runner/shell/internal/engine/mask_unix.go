//go:build darwin || linux

package engine

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runner/process"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) umask(ctx context.Context, args []string) error {
	handler := interp.HandlerCtx(ctx)
	state := handler.Scope().(*interpreterScope)
	reusable, symbolic := false, false
	mode := ""
	for _, arg := range args[1:] {
		if mode == "" && strings.HasPrefix(arg, "-") && strings.Trim(arg[1:], "pS") == "" {
			reusable = reusable || strings.Contains(arg, "p")
			symbolic = symbolic || strings.Contains(arg, "S")
		} else if mode == "" {
			mode = arg
		} else {
			return diagnostic(ctx, 2, "umask: %s: too many arguments\n", arg)
		}
	}
	current := uint32(0)
	if state.attributes.Umask != nil {
		current = *state.attributes.Umask
	} else {
		current = process.Umask()
	}
	if mode != "" {
		mask, err := parseMask(ctx, current, mode)
		if err != nil {
			return err
		}
		state.attributes.Umask = &mask
		current = mask
		if !symbolic {
			return nil
		}
	}
	shown := fmt.Sprintf("%04o", current)
	if symbolic {
		who := func(shift uint) string {
			var value strings.Builder
			for i, c := range "rwx" {
				if (^current>>shift)&(4>>i) != 0 {
					value.WriteRune(c)
				}
			}
			return value.String()
		}
		shown = fmt.Sprintf("u=%s,g=%s,o=%s", who(6), who(3), who(0))
	}
	prefix := ""
	if reusable {
		prefix = "umask "
		if symbolic {
			prefix = "umask -S "
		}
	}
	_, err := fmt.Fprintln(handler.Stdout, prefix+shown)
	return err
}

// symbolicPermissions applies a shell mask's symbolic permission clauses.
func symbolicPermissions(current uint32, mode string) (uint32, error) {
	for _, clause := range strings.Split(mode, ",") {
		who, i, err := permissionClasses(clause)
		if err != nil {
			return 0, err
		}
		for i < len(clause) {
			op := clause[i]
			i++
			if op != '+' && op != '-' && op != '=' {
				return 0, fmt.Errorf("invalid mode")
			}
			bits, next, err := permissionBits(current, clause, i)
			if err != nil {
				return 0, err
			}
			i = next
			bits &= who
			switch op {
			case '+':
				current |= bits
			case '-':
				current &^= bits
			case '=':
				current = current&^who | bits
			}
		}
		if i == 0 {
			return 0, fmt.Errorf("invalid mode")
		}
	}
	return current, nil
}

func parseMask(ctx context.Context, current uint32, mode string) (uint32, error) {
	var mask uint32
	if mode[0] >= '0' && mode[0] <= '9' {
		value, err := strconv.ParseUint(mode, 8, 32)
		if err != nil || value > 0o777 {
			return 0, diagnostic(ctx, 1, "umask: %s: octal number out of range\n", mode)
		}
		mask = uint32(value)
	} else {
		allowed, err := symbolicPermissions(^current&0o777, mode)
		if err != nil {
			return 0, diagnostic(ctx, 1, "umask: %s: %v\n", mode, err)
		}
		mask = ^allowed & 0o777
	}
	return mask, nil
}

func permissionBits(current uint32, clause string, i int) (uint32, int, error) {
	bits := uint32(0)
	for i < len(clause) && clause[i] != '+' && clause[i] != '-' && clause[i] != '=' {
		switch clause[i] {
		case 'r':
			bits |= 0o444
		case 'w':
			bits |= 0o222
		case 'x':
			bits |= 0o111
		case 'X':
			if current&0o111 != 0 {
				bits |= 0o111
			}
		case 'u', 'g', 'o':
			shift := uint(0)
			switch clause[i] {
			case 'u':
				shift = 6
			case 'g':
				shift = 3
			}
			p := (current >> shift) & 7
			bits |= p | (p << 3) | (p << 6)
		default:
			return 0, i, fmt.Errorf("invalid mode")
		}
		i++
	}
	return bits, i, nil
}

func permissionClasses(clause string) (uint32, int, error) {
	i := 0
	who := uint32(0)
	for i < len(clause) {
		var bits uint32
		switch clause[i] {
		case 'u':
			bits = 0o700
		case 'g':
			bits = 0o070
		case 'o':
			bits = 0o007
		case 'a':
			bits = 0o777
		}
		if bits == 0 {
			break
		}
		who |= bits
		i++
	}
	if i == len(clause) {
		return 0, i, fmt.Errorf("invalid mode")
	}
	if who == 0 {
		who = 0o777
	}
	return who, i, nil
}
