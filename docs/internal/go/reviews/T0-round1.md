# T0 verification, round 1 (branch go/wp/T0-corpora at eadee144): do not merge

Orchestrator decisions (binding):
- D1 Directory order: Go utilities walk directories in name order (docs/demi-next/runner.md,
  commit 37282cf5). Cases whose GNU output order follows the filesystem (find, grep -r, du,
  rg without --sort) are marked `unordered` and compared as a multiset of lines; ls -S must not
  depend on directory sizes.
- D2 Stdin: toolctx.Invocation.StdinKind (none | pipe | file), commit 37282cf5. A case states its
  stdin kind; "none" is recorded with </dev/null. `rg PATTERN` with no input must be in core.
- D3 gnucorpus.Check takes the full utility registry; the harness supplies echo, true, false and
  printf test doubles (shell builtins in the product) for find -exec and xargs.

High findings to fix:
- H1 order-dependent cases (search.go:77-94 find, search.go:29 grep -r, fs.go:167-171 du,
  fs.go:26 ls -S): apply D1.
- H2 BaseEnv copies the host PATH (gnucorpus.go:114); env cases recorded it. Pin PATH to a fixed
  value in BaseEnv.
- H3 core must cover every option the old tests use, with real input: tail -c +1
  (runner/tests/tasks.rs:281), seq -f '%09g' (host-remote jobs.test.ts:172), find -exec … ';' and
  -exec sh '{}' + and -maxdepth 0 (tasks.rs:341-342), sort sorted -o sorted (edit_tracking.rs:47),
  tr a-z A-Z ranges (shell.rs:56, pipes.test.ts:106), wc -c on stdin (tasks.rs:127/279,
  pipes.test.ts:176), head -n1 (jobs.test.ts:143), cat -- path (remote-files.test.ts:66), sed
  labels/branches ':again; b again' and the e command (tasks.rs:275/343). For what the corpus
  cannot record (tail -f -s 60, successful mktemp, df -h), README names the WP test that covers it.
- H4 core must cover what coding agents commonly use; missing today:
  cat: no operand (stdin) | head: -nN/-cN attached, --lines=N, stdin | tail: -20, -nN, -c +N |
  wc: -l/-c on stdin, -m | sort: -rn/-nr, -k2 without -t, -V, -h, -s, -o onto input, -u with -k |
  cut: default TAB, open ranges -f2- -c-5, attached -d: -f1, -d' ' | tr: ranges, escapes '\n',
  -d '\n', -cd, -d '[:space:]' | grep: -q -e -o -w -A/-B/-C, --include/--exclude/--exclude-dir,
  -h/-H, -m, -rn/-rl/-in, -R, BRE \|, -s | rg: no path (cwd), --files, -g/--glob, -t/--type, -S,
  -w, -o, -A/-B/-C, --hidden, --no-ignore/-u, -e, -U, -m, -N/--no-heading, .gitignore/.ignore |
  find: -path, -prune -o … -print, -delete, -exec, -type f -name, -size, -newer/-mtime (pinned
  times), -print, !/-not, -regex, -L, time-free -printf | xargs: -r, -d '\n', -t, -P 1, sh -c,
  -n1 -I | sed: -E, regex addresses /re/p /a/,/b/d, other delimiters s|a|b|, &, a\ i\ c\, y, q, =,
  N/P/D, -i -e, -z | jq: file operand, .[] and .[] | .f, -e, --argjson, -S, -j, -R, --slurpfile,
  @csv/@tsv, //, to_entries, has, interpolation | ls: -l -la -lh -t -r (pinned times, as root) |
  cp: -R -f -v -a/-p, files into dir, -r src/ dst, -r into existing dir, -t | mv: -f -v, into dir,
  -t | rm: -v -d -- , -r missing | mkdir: -v -m | touch: -a -d | stat: %Y %U %G, --printf, -L |
  du: -h, -sh --apparent-size, -d N, -c | df: -h and . (normalized or covered elsewhere) |
  chmod: +x, 755, u=rwx,go=rx, -v/-c | chown: root:root, -h, -R | realpath: -m/-s, absolute |
  basename: suffix form | env: -C | seq: -f -w fractional | date: -r FILE, -I, -R, +%H:%M:%S,
  -d '2024-01-01 +1 day' | paste: -sd, and - - | od: -x -A x -v -w | diff: -r/-ru -N -U N
  -w/-b/-B/-i --label -y stdin - | cmp: -b, stdin -.
- H5 extended sets must really be "the rest of the documented options worth supporting";
  today 92 cases (mv 0, sleep 0, diff 2 …).

Medium findings to fix:
- M1 owners/modes are compared for every path (gnucorpus.go:264-274, 355-373; README:47 says
  otherwise): record them only where a case asks (chmod, chown, cp -p, mkdir -m, touch). Input
  trees are written before the umask is set (gnucorpus.go:416-421, record/main.go:90-95), and
  Check changes the process umask (races between parallel tests): create trees under the fixed
  umask explicitly and use toolctxtest.Runner.Umask.
- M2 record/main.go:108-115 records a killed reference (timeout) as exit -1: fail the recording.
- M3 mislabeled or empty cases: tr delete-and-squeeze (text.go:278) records a usage error;
  tail dash-c-plus-from-start has empty input; sort merge-two-files merges unsorted input;
  find print0-then-count counts nothing; head dash-n-space-count runs -3.
- M4 Check needs declared known gaps (with reasons), allowed only for extended cases; a core
  mismatch always fails; the harness reports core and extended pass counts.
- M5 add the stdin kind (D2) and normalization placeholders (the case's work dir path, random
  names) so mktemp, plain realpath and df can be tested.
- M6 harness self-tests must show that a wrong exit code, stderr presence, extra/missing file and
  wrong mode/owner are each detected; a test loads all 41 corpora; Load rejects unknown fields,
  empty/duplicate names and argv containing the utility's own name.

Low: use slices.Contains, maps.Copy, slices.Sorted(maps.Keys(...)) instead of hand-written
copies (gnucorpus.go:386, 121, text.go:232); testdataDir must not depend on runtime.Caller
(breaks with -trimpath); definitions.Order duplicates Registry keys; Check runs cases without a
timeout (give each a deadline); extended cases as nested subtests is fine.

Replay proof the verifier used (reuse it): a throwaway test that runs Check for every utility
with a stand-in utility exec-ing the GNU binary must pass on ext4 and on TMPDIR=/dev/shm, with a
different PATH, and under umask 077.
