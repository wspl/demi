package skills

import (
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
)

// Front matter scenarios protect the lenient Agent Skills boundary; all run
// in memory in under one second, including the exact 256 KiB limit.
func TestSkillFrontMatter(t *testing.T) {
	for _, test := range []struct {
		name, directory, text string
		want                  parsedSkill
		failure               string
	}{
		{name: "ordinary", directory: "review", text: "---\nname: review\ndescription: Review a change.\n---\n# body", want: parsedSkill{name: "review", description: "Review a change.", warnings: []string{}}},
		{name: "missing name", directory: "Bad_Name", text: "---\ndescription: Review.\n---", want: parsedSkill{name: "Bad_Name", description: "Review.", warnings: []string{}}},
		{name: "unknown fields", directory: "review", text: "---\ndescription: Review.\nmetadata:\n  nested: [anything, true]\nlicense: MIT\n---", want: parsedSkill{name: "review", description: "Review.", warnings: []string{}}},
		{name: "BOM CRLF and multiline", directory: "review", text: "\ufeff--- \r\nname: review\ndescription: |\n  First line.\n  Second line.\ndisable-model-invocation: true\n---\r\n", want: parsedSkill{name: "review", description: "First line.\nSecond line.\n", disableModelInvocation: true, warnings: []string{}}},
		{name: "warnings", directory: "directory", text: "---\nname: Bad_Name\ndescription: Review.\n---", want: parsedSkill{name: "Bad_Name", description: "Review.", warnings: []string{`the name "Bad_Name" is not 1 to 64 lowercase letters, digits and single hyphens`, `the name "Bad_Name" differs from its directory "directory"`}}},
		{name: "missing description", directory: "review", text: "---\nname: review\n---", failure: "the front matter has no description"},
		{name: "blank description", directory: "review", text: "---\ndescription: '  '\n---", failure: "the front matter has no description"},
		{name: "null optional fields", directory: "review", text: "---\nname: null\ndescription: Review.\ndisable-model-invocation: null\n---", want: parsedSkill{name: "review", description: "Review.", warnings: []string{}}},
		{name: "numeric spelling", directory: "001", text: "---\nname: 001\ndescription: 1.00\n---", want: parsedSkill{name: "001", description: "1.00", warnings: []string{}}},
		{name: "quoted boolean", directory: "review", text: "---\ndescription: Review.\ndisable-model-invocation: 'true'\n---", want: parsedSkill{name: "review", description: "Review.", disableModelInvocation: true, warnings: []string{}}},
		{name: "YAML11 boolean", directory: "review", text: "---\ndescription: Review.\ndisable-model-invocation: YES\n---", want: parsedSkill{name: "review", description: "Review.", disableModelInvocation: true, warnings: []string{}}},
		{name: "alias", directory: "review", text: "---\nname: &name review\ndescription: *name\n---", want: parsedSkill{name: "review", description: "review", warnings: []string{}}},
		{name: "not YAML", text: "---\ndescription: [unclosed\n---", failure: "the front matter does not parse:"},
		{name: "duplicate", text: "---\ndescription: First.\ndescription: Second.\n---", failure: "the front matter does not parse:"},
		{name: "wrong bool", text: "---\ndescription: Review.\ndisable-model-invocation: maybe\n---", failure: "the front matter does not parse:"},
		{name: "sequence description", text: "---\ndescription: [review]\n---", failure: "the front matter does not parse:"},
		{name: "not UTF8", text: "---\ndescription: \xff\n---", failure: "SKILL.md is not UTF-8"},
		{name: "no delimiters", text: "description: Review.", failure: "SKILL.md has no front matter"},
		{name: "leading newline", text: "\n---\ndescription: Review.\n---", failure: "SKILL.md has no front matter"},
		{name: "no closing delimiter", text: "---\ndescription: Review.\n", failure: "SKILL.md has no front matter"},
	} {
		t.Run(test.name, func(t *testing.T) {
			got, err := parseSkill(test.directory, []byte(test.text))
			if test.failure != "" {
				if err == nil || !strings.HasPrefix(err.Error(), test.failure) {
					t.Fatalf("got %v, want %q", err, test.failure)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if diff := cmp.Diff(test.want, got, cmp.AllowUnexported(parsedSkill{})); diff != "" {
				t.Fatal(diff)
			}
		})
	}
}

func TestSkillLimits(t *testing.T) {
	base := "---\ndescription: Review.\n---\n"
	for _, size := range []int{skillMDMaxBytes, skillMDMaxBytes + 1} {
		_, err := parseSkill("review", []byte(base+strings.Repeat("x", size-len(base))))
		if (err != nil) != (size > skillMDMaxBytes) {
			t.Fatalf("size %d: %v", size, err)
		}
	}
	for _, length := range []int{1024, 1025} {
		skill, err := parseSkill("review", []byte("---\ndescription: "+strings.Repeat("雪", length)+"\n---"))
		if err != nil {
			t.Fatal(err)
		}
		if (len(skill.warnings) > 0) != (length > 1024) {
			t.Fatalf("description length %d: %v", length, skill.warnings)
		}
	}
}
