package main

import (
	"go/types"
	"regexp"
	"strings"
)

var nameWords = regexp.MustCompile(`[A-Z]+[a-z]*|[a-z]+|[0-9]+`)

// words separates Go initialisms from the following contract name word.
func words(name string) []string {
	var out []string
	for _, word := range nameWords.FindAllString(name, -1) {
		boundary := 0
		for i := 1; i < len(word); i++ {
			if word[i] >= 'a' && word[i] <= 'z' {
				if i > 1 {
					boundary = i - 1
				}
				break
			}
		}
		if boundary > 0 {
			out = append(out, word[:boundary])
			word = word[boundary:]
		}
		out = append(out, word)
	}
	return out
}

func tsName(name string) string {
	parts := words(name)
	for i, part := range parts {
		parts[i] = strings.ToUpper(part[:1]) + strings.ToLower(part[1:])
	}
	return strings.Join(parts, "")
}

func isPrivateScalar(d *definition) bool {
	_, basic := d.typ.Underlying().(*types.Basic)
	return (basic || has(d.marks, "base64")) && !has(d.marks, "enum")
}
