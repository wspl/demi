//go:build darwin || linux

package shell

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runner/process"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) umask(ctx context.Context, args []string) error {
	hc := interp.HandlerCtx(ctx)
	state := hc.Scope().(*interpreterScope)
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
		var mask uint32
		if mode[0] >= '0' && mode[0] <= '9' {
			value, err := strconv.ParseUint(mode, 8, 32)
			if err != nil || value > 0777 {
				return diagnostic(ctx, 1, "umask: %s: octal number out of range\n", mode)
			}
			mask = uint32(value)
		} else {
			allowed, err := symbolicPermissions(^current&0777, mode)
			if err != nil {
				return diagnostic(ctx, 1, "umask: %s: %v\n", mode, err)
			}
			mask = ^allowed & 0777
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
	_, err := fmt.Fprintln(hc.Stdout, prefix+shown)
	return err
}

// symbolicPermissions applies a shell mask's symbolic permission clauses.
func symbolicPermissions(current uint32, mode string) (uint32, error) {
	for _, clause := range strings.Split(mode, ",") {
		i := 0
		who := uint32(0)
		for i < len(clause) {
			var bits uint32
			switch clause[i] {
			case 'u':
				bits = 0700
			case 'g':
				bits = 0070
			case 'o':
				bits = 0007
			case 'a':
				bits = 0777
			}
			if bits == 0 {
				break
			}
			who |= bits
			i++
		}
		if who == 0 {
			who = 0777
		}
		for i < len(clause) {
			op := clause[i]
			i++
			if op != '+' && op != '-' && op != '=' {
				return 0, fmt.Errorf("invalid mode")
			}
			bits := uint32(0)
			for i < len(clause) && clause[i] != '+' && clause[i] != '-' && clause[i] != '=' {
				switch clause[i] {
				case 'r':
					bits |= 0444
				case 'w':
					bits |= 0222
				case 'x':
					bits |= 0111
				case 'X':
					if current&0111 != 0 {
						bits |= 0111
					}
				case 'u', 'g', 'o':
					shift := uint(0)
					if clause[i] == 'u' {
						shift = 6
					} else if clause[i] == 'g' {
						shift = 3
					}
					p := (current >> shift) & 7
					bits |= p | (p << 3) | (p << 6)
				default:
					return 0, fmt.Errorf("invalid mode")
				}
				i++
			}
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
