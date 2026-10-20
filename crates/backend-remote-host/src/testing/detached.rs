//! The runners a test starts in sessions of their own, as an installer and
//! `run start` start them (`crates/runner/src/console.rs`):
//! no kill of the test's process tree or group reaches them, so a test that
//! ends without stopping them, or is killed, leaves them running for good.
//! [`DetachedRunners`] stops them when it drops, and when the test process
//! ends without dropping it.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, PoisonError};

/// The watchdog: it reads the installation directories to watch, a line each,
/// until a line `stop`, which the guard writes as it drops, or the end of its
/// input, which comes when the test process ends however it ends.
///
/// Then it finds each installation's runners as they are at that moment, stops
/// them, and says `stopped`. A process is a runner of an installation when it
/// is one of these:
///
/// - The process that listens on the local endpoint the installation's
///   `active.json` names.
/// - When the runner has not written that yet or no longer has it, the process
///   that holds the installation's `runner.lock`, which the runner takes before
///   it writes `active.json` and keeps until it ends. `lsof` names both, with
///   no help of the runner's.
/// - A process whose command line names the installation, such as a runner that
///   has not taken the lock yet when a killed test leaves it to the watchdog.
///   The other two miss it: it took the lock after the watchdog's last round
///   and ran on for good.
///
/// Each is sent `TERM`, which ends its jobs as a stop does, and `KILL` once ten
/// seconds have passed. It is signalled only while its command line still names
/// its installation, so neither a process that took its id after it ended nor a
/// test's own `lsof` match is stopped.
///
/// A runner that replaced itself with an update as it was stopped is found and
/// stopped in the next round, until a round finds none.
///
/// The watchdog ignores the signals that end the test, and runs in a process
/// group of its own, outside the test's process tree once its first shell
/// exits, so that what stops the test does not stop it first.
///
/// The guard asks and waits by line, not by closing pipes: a process another
/// test starts at the moment a pipe is made may inherit its end, and would hold
/// it open.
///
/// The input reaches the background block through descriptor 3: dash, Linux's
/// `sh`, gives an asynchronous list `/dev/null` as its input even when the list
/// redirects `<&0` itself, so the watchdog read nothing, ended at once, and
/// every later `watch` met a broken pipe.
const WATCHDOG: &str = r#"exec 3<&0
{
trap '' HUP INT TERM
installations=
while IFS= read -r line; do
  [ "$line" = stop ] && break
  installations="$installations
$line"
done
ours() {
  case "$(ps -ww -o command= -p "$1" 2>/dev/null)" in
    *"$2"*) return 0 ;;
    *) return 1 ;;
  esac
}
# The processes of the installation "$1": its endpoint's listener and its
# lock's holder, a line "<pid> <installation>" each.
runners() {
  endpoint=$(sed -n 's/.*"endpoint":"\([^"]*\)".*/\1/p' "$1/active.json" 2>/dev/null)
  {
    if [ -n "$endpoint" ]; then
      lsof -U -Fpn 2>/dev/null | awk -v want="$endpoint" '
        /^p/ { pid = substr($0, 2) }
        /^n/ { name = substr($0, 2); if (name == want || name == "/private" want) print pid }'
    fi
    [ -e "$1/runner.lock" ] && lsof -t "$1/runner.lock" 2>/dev/null
    # The grep itself is listed too, and has ended by the check below.
    ps -A -ww -o pid=,command= | grep -F -- "$1" | awk '{ print $1 }'
  } | sort -u | while read -r pid; do
    ours "$pid" "$1" && printf '%s %s\n' "$pid" "$1"
  done
}
round=0
while [ "$round" -lt 5 ]; do
  round=$((round + 1))
  found=$(printf '%s\n' "$installations" | while IFS= read -r installation; do
    [ -n "$installation" ] && runners "$installation"
  done)
  [ -n "$found" ] || break
  printf '%s\n' "$found" | while read -r pid installation; do
    kill -TERM "$pid" 2>/dev/null
  done
  printf '%s\n' "$found" | while read -r pid installation; do
    waited=0
    while ours "$pid" "$installation"; do
      if [ "$waited" -ge 200 ]; then
        kill -KILL "$pid" 2>/dev/null
        break
      fi
      sleep 0.05
      waited=$((waited + 1))
    done
  done
done
echo stopped
} <&3 3<&- &"#;

/// Stops the runners of the installations it watches when it drops, on a
/// test's success, failure or panic, and when the test process is killed;
/// it finds them only then, so a runner started at any moment before is
/// stopped.
pub struct DetachedRunners {
    /// The watchdog's input, until the guard asks it to stop.
    input: Mutex<Option<ChildStdin>>,
    watchdog: Child,
}

impl DetachedRunners {
    pub fn new() -> Self {
        use std::os::unix::process::CommandExt as _;
        let mut watchdog = Command::new("sh")
            .arg("-c")
            .arg(WATCHDOG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("the runners' watchdog starts");
        // Taken first: waiting closes a child's input.
        let input = watchdog.stdin.take();
        // Its first shell exits once it started the watchdog proper.
        watchdog.wait().expect("the runners' watchdog starts");
        Self {
            input: Mutex::new(input),
            watchdog,
        }
    }

    /// Watches the installation in `state`, which need not exist yet: its
    /// runners are stopped with the guard.
    pub fn watch(&self, state: &Path) {
        if let Some(input) = self.input.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            writeln!(input, "{}", state.display()).expect("the runners' watchdog listens");
        }
    }
}

impl Default for DetachedRunners {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DetachedRunners {
    fn drop(&mut self) {
        let asked = self
            .input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .is_some_and(|mut input| input.write_all(b"stop\n").is_ok());
        // A watchdog that is gone stopped what it found already.
        if let (true, Some(output)) = (asked, self.watchdog.stdout.take()) {
            let _ = BufReader::new(output).read_line(&mut String::new());
        }
    }
}
