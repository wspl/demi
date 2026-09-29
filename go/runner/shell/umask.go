package shell

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode"
)

// parseMask implements the process builtin's allowed-permission expression.
// This is the uucore parse_chmod behavior used by Rust's umask, not a utility.
func parseMask(current uint32, mode string) (uint32, error) {
	if len(mode) > 0 && mode[0] >= '0' && mode[0] <= '9' {
		mask, err := strconv.ParseUint(mode, 8, 32)
		if err != nil || mask > 0777 {
			return 0, errors.New("octal number out of range")
		}
		return uint32(mask), nil
	}
	allowed := ^current & 0777
	for clause := range strings.SplitSeq(mode, ",") {
		clause = strings.TrimSpace(clause)
		if clause == "" {
			continue
		}
		if strings.ContainsFunc(clause, unicode.IsDigit) {
			op := byte('=')
			digits := clause
			if strings.ContainsRune("+-=", rune(clause[0])) {
				op, digits = clause[0], strings.TrimSpace(clause[1:])
			}
			value, err := strconv.ParseUint(digits, 8, 32)
			if err != nil {
				return 0, errors.New("invalid digit found in string")
			}
			if value > 07777 {
				return 0, fmt.Errorf("mode is too large (%o > 7777)", value)
			}
			switch op {
			case '+':
				allowed |= uint32(value)
			case '-':
				allowed &^= uint32(value)
			case '=':
				allowed = uint32(value)
			}
			continue
		}
		var who uint32
		i := 0
		for i < len(clause) {
			bit := map[byte]uint32{'u': 04700, 'g': 02070, 'o': 01007, 'a': 07777}[clause[i]]
			if bit == 0 {
				break
			}
			who |= bit
			i++
		}
		if i == len(clause) {
			return 0, fmt.Errorf("invalid mode (%s)", clause)
		}
		if i == 0 {
			who = 07777
		}
		for i < len(clause) {
			op := clause[i]
			if !strings.ContainsRune("+-=", rune(op)) {
				return 0, fmt.Errorf("invalid operator (expected +, -, or =, but found %c)", op)
			}
			i++
			var bits uint32
			start := i
			for i < len(clause) {
				c := clause[i]
				switch c {
				case 'r':
					bits |= 0444
				case 'w':
					bits |= 0222
				case 'x':
					bits |= 0111
				case 'X':
					if allowed&0111 != 0 {
						bits |= 0111
					}
				case 's':
					bits |= 06000
				case 't':
					bits |= 01000
				case 'u', 'g', 'o':
					shift := map[byte]uint{'u': 6, 'g': 3, 'o': 0}[c]
					value := (allowed >> shift) & 7
					bits = value | value<<3 | value<<6
				default:
					goto applied
				}
				if c == 'u' || c == 'g' || c == 'o' {
					if i == start {
						i++
					}
					break
				}
				i++
			}
		applied:
			switch op {
			case '+':
				allowed |= bits & who
			case '-':
				allowed &^= bits & who
			case '=':
				allowed = allowed&^who | bits&who
			}
		}
	}
	return ^allowed & 0777, nil
}
