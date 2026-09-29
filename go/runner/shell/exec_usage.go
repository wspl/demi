package shell

import "fmt"

// execArguments preserves the Rust builtin's clap usage contract, including
// treating an unrecognized hyphen-prefixed word as the program operand.
func execArguments(args []string) []string {
	result := []string{"exec"}
	seen := make(map[string]bool)
	for i := 0; i < len(args); i++ {
		option := args[i]
		switch option {
		case "-h", "--help":
			return []string{"__demi_usage", execHelp + "\n"}
		case "-a", "-c", "-l":
			if seen[option] {
				label := option
				if option == "-a" {
					label = "-a <NAME>"
				}
				text := fmt.Sprintf("error: the argument '%s' cannot be used multiple times\n\nUsage: exec [OPTIONS] [COMMAND]...\n\nFor more information, try '--help'.\n\n", label)
				return []string{"__demi_usage", text}
			}
			seen[option] = true
			result = append(result, option)
			if option == "-a" {
				if i+1 == len(args) {
					return []string{"__demi_usage", "error: a value is required for '-a <NAME>' but none was supplied\n\nFor more information, try '--help'.\n\n"}
				}
				i++
				result = append(result, args[i])
			}
		case "--":
			return append(result, args[i:]...)
		default:
			return append(append(result, "--"), args[i:]...)
		}
	}
	return result
}

const execHelp = "`exec`: with a command, the command runs as the shell's last, and the shell (a subshell's, or the\n" +
	"job's) then ends with its status; bash would replace the process, which here is the runner. With\n" +
	"only redirections, they stay with the shell, as in bash\n\n" +
	"Usage: exec [OPTIONS] [COMMAND]...\n\nArguments:\n  [COMMAND]...  The command and its arguments\n\nOptions:\n" +
	"  -a <NAME>   Pass NAME to the command as its zeroth argument\n" +
	"  -c          Run the command with an empty environment\n" +
	"  -l          Run the command as a login shell, with a dash before its zeroth argument\n" +
	"  -h, --help  Print help\n"
