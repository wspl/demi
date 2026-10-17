# The shell tool

How an agent runs commands: the `shell` tool's input, how a call watches its command and returns, the `demi shell` commands, results, previews and media, failures, interrupted calls and lost commands, and the reports a running command sends its agent. Sessions, turns and input are in [Agent runtime](runtime.md).

## Command reports

For example, the model starts the test suite, command 17, with
`intervalMs: 300000`. The suite runs on after a minute, so the call returns
its handle and output, saying that the command moved to the background and
reports every 5 minutes, and the model tells the user that the tests are
running and ends its turn: the user can talk to the agent meanwhile.
Five minutes after the start the node is woken with a report, "Command 17 is still
running; no output for 4 minutes 50 seconds", and the newest lines. The model
sees that the suite hangs in one test, stops it and runs that test alone.
Had the suite ended first, the node would have been woken with "Command 17
(Run the test suite) ended with exit code 1" and the suite's last lines, as
the call's result would have shown them. The model never
asks to be woken: as Claude Code wakes its model when a background command
or a subagent ends, the work it waits for wakes it.

- **Every command says how it is watched.** `shell` takes
  `intervalMs`, a number or `null`, and no default, so the model decides when
  it starts the command:
  - **A number**, from 15,000 to 600,000 milliseconds, for a command that
    ends, such as a build or a test suite: how often the command reports
    while it runs, short for one that should end within minutes, long for
    one that takes long, since each report is a request of the whole
    conversation. The call watches the command for a minute, or for its
    interval when that is shorter; if it still runs, the call returns its
    handle and output so far with the line `[The command is still running,
    so it moved to the background. It reports every 5m until it ends, and
    when it ends.]`, and the node then gets a report every interval from
    the command's start until it ends. The minute is not the model's to
    set and no tool text mentions it: a call never holds the turn longer,
    so a user's steer waits at most a minute for the model to read it,
    where a ten-minute interval used to hold it ten.
     A report
    carries what `demi shell status` would show: the output since the
    model's last look, how long the command has printed nothing, and how to
    change the interval. A smaller value is taken as 15,000 and a larger one
    as 600,000, and the result says so.
  - **`null`** for a command that runs until it is stopped, such as a dev
    server or a watcher. The call returns once the command's output has been
    quiet for 2 seconds, or after 30 seconds, so the model sees it start,
    and the node gets no reports while it runs.
  - `demi shell status --interval` and `--resident` change a running
    command's interval from then on: longer when a report shows the command needs
    more time, `null` when the model learns that a command it started with a
    number serves, a number when it wants to hear from a resident one. A
    report says how, so the model changes it where it already looks at the
    command.
- **Every end is reported, with what the model would look at.** A command
  whose call returned while it still ran tells its node when it ends,
  however it ends: an exit with its status,
  a stop by the user or another agent, or a loss with its reason
  ([Lost commands](#lost-commands)). A command the node stopped itself with
  `demi shell stop`, which printed `[command 17 stopped]`, and one whose end
  the model saw in a result already, tell nothing more. An end report is the
  result that reports the command's end
  ([What a result attaches](#what-a-result-attaches)): it carries the output
  since the model's last look, bounded as a `shell` result is, and the media
  the job viewed, so the model needs no `demi shell status` to see how it
  ended: in a product check, a model woken by an end without its output
  spent one more request on a look every time.
- **How a report arrives.** A report joins a turn that accepts steers at its
  next continuation boundary, as a `wakeup` block with the placement
  `steer`; otherwise it starts a continuation whose input is a `wakeup` block
  with the placement `new_turn`. Each report names the command by its
  number and its call's title, as Claude Code's notice names a background
  command by its description, which the command's record keeps from its start,
  so a report has its title whatever became of the call or the process that
  watched it, a lost command's too, since a model may have several running:
  `Command 17 (Run the test suite) ended with exit code 1.`, followed by its
  output as a result shows it. Reports that arrive together are one block,
  one paragraph each: `Command 18 was stopped by the user.`,
  `Command 19 was lost: Demi was upgraded and the Host's runner replaced
  itself. Start it again if it is still needed.` A report never appears in
  the queue or among the pending steers, and a running call's window ends
  when one arrives ([The window](#the-window)), except the end report of a
  command that call watches or looks at, whose end the call shows itself. A
  report moves the model's place in the command's output as a look does,
  when it is written into the transcript; an end report dropped because a
  look showed the end first moves nothing, so the look shows the last
  lines. An end report whose job viewed media is never dropped: it carries
  the media, without the output the look showed.
- **A Host that is away.** A command whose Host's connection is away sends
  no interval report: the node could do nothing with one, and its silence
  is the connection's, not the command's. Its reports resume when the
  runner is back, and its end is reported as any end ([Command lifetime](../execution/runner.md#command-lifetime)).
  A look at it with `demi shell status` from another Host says that its
  Host is unreachable, and since when.
- **Across a restart.** A command's start time and each node's place in its
  output are part of the record the backend keeps of a running command, so
  a report after a backend restart counts the time from the command's
  start and shows only output the model has not seen.
- **What the user sees.** The transcript shows each end report as a notice
  row where its block lies, as Claude Code shows a line when a background
  task ends, so a reply the agent writes after a command's end never
  appears without its cause. For example, the model starts the suite, says
  it will report back and ends its turn; when the suite ends, a row with a
  bell reads *Command failed with exit code 1: Run the test suite*, and
  under it Requesting stands until the model's answer arrives, as under a
  message the user sent. The notice says what happened, in the user's
  words, and then names the call by its title: *Command finished*, *Command
  stopped by you*, *Command stopped by another agent*, *Lost track of
  command*, never *Command lost*, which read as a command that had just come
  back. A
  title is an imperative, so it is the notice's object, never its subject:
  joined as the subject, it read wrongly in the product, *Restart the
  end-to-end suite is still running*. A failure, a stop and a loss carry the
  same tag a shell row carries ([Rendering boundary](rendering.md#rendering-boundary)).
  The title is underlined with dots, and a click brings the call that
  started the command into view and marks it for a moment, as a search
  result does, reading the page around it when the page does not hold it
  ([Transcript windows](../product/web-application.md#transcript-windows)):
  the call's row is where the command's script and output are. A call
  folded inside a run is never stood in for by the run's row: the run opens,
  and the call's own row comes into view and is marked, as a file opens its
  folders in a tree to show a search result. The same holds for every reveal
  of a block, a search result's and a receipt's sender's included. The run
  stays open after, as the reader's own opening would leave it. A report of
  progress shows nothing, and a block of progress reports alone has no row:
  the answer it wakes says what the model learned, and a row for every
  interval would outnumber the answers it caused. So the block holds each report as data, its
  `commandId`, the call's `title`, its `event` (`running`, `ended` with
  its exit code, `stopped` with who stopped it, or `lost` with its reason),
  and the output and media it carried, and the text the model reads is
  rendered from them in one place, never stored beside them. A report that carries media shows them under its row,
  as a call's result does
  ([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
- **Waiting is ending the turn.** A model that has nothing left to do but
  wait for a command or a subagent ends its turn, with a reply that says
  what it waits for, since the user reads only what the model wrote. The
  tool rules say so, and say not to poll. To come back after a while for
  something no command stands for, such as a deployment that becomes
  visible minutes later, the model starts `sleep 600` with `intervalMs:
  null`, so the call returns at once, and ends its turn; the sleep's end
  wakes it. Started with a number instead, the call would watch the sleep
  for that long and hold the turn, and the user would watch the agent wait.
  A script never sleeps a fixed time before it looks for something: it waits
  on the thing itself, such as the line it expects in a log
  (`tail -f app.log | grep -m1 ready`) or the command whose end it needs. A
  command that must end by a deadline runs under `timeout`
  ([Background tasks and timeouts](../execution/runner.md#background-tasks-and-timeouts)).
- **Restarts.** A report that has arrived but is not yet in the transcript
  is saved in the checkpoint, as agent messages are, and so is each running
  command's interval, so both survive dispose and a backend restart. A
  command that reports while its conversation's tree is not live, such as
  one the backend took up again after a restart
  ([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)),
  restores the tree, as an `open` does, to tell its node. A root whose last
  turn was interrupted ended it with an `error` block, so it holds its
  reports and its agent input as any failed root does
  ([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)).
- **Children.** A subagent whose background commands still run stays live,
  as one with live children does, and closes once they have ended and it
  has answered ([Result](subagents.md#result)).

## Tools

The model has one tool, `shell`, and only it:

| Tool | What it does |
| --- | --- |
| `shell` | Starts a script in a shell on the conversation's Host and watches it as its `intervalMs` says ([Command reports](#command-reports)). The window is for watching, not a deadline: a command still running when it ends keeps running, the result carries its handle (`commandId`), and the node hears from it again as its interval says. Completed short output returns directly. |

A tool is only what the shell cannot do: start a script and watch it. Looking
at a command again, writing to its input and stopping it are things a shell
does, so they are the commands `demi shell status`, `demi shell input` and
`demi shell stop` ([The `demi shell` commands](#the-demi-shell-commands)),
which a script can combine with anything else. Ending a turn needs no tool,
since a response that calls none ends it, and waiting needs none, since the
work's end wakes the node ([Command reports](#command-reports)). Earlier
designs had three tools: `shell_status` to look at a command, which a
command does as well, and `yield`, which ended the turn and set a timer;
models used the timer to wait for work that wakes them anyway, and a timer
that fired after the work had ended opened a turn that only said so. Claude
Code likewise has no tool to look at a background command: its output is a
file the model reads like any other.

Everything else the agent does runs as commands in the shell
([Commands](../execution/commands.md)). The runtime grafts its own groups,
`demi agent` ([Subagents](subagents.md)) and `demi shell`
([The `demi shell` commands](#the-demi-shell-commands)), into each node's
command set; the others, such as `demi file`, `demi browser` and `demi
host`, come from the command set the product supplies. A tool call whose name
is not `shell` completes as an error `Tool not found: <name>`.

### Tool input

- Each tool declares its input once. The JSON Schema the model receives and the
  check each call runs come from that one declaration. The schema's dialect
  declaration is dropped, because a provider embeds the schema in its own
  request.
- Unknown fields and wrong types are refused. A refused call completes as an
  error whose text starts with `<tool> input is invalid:` and names each
  offending field, so the model can correct the call.
- `intervalMs` is required: whole milliseconds, or `null`, declared to the
  model as `integer`, whose description gives its bounds, 15,000 to 600,000.
  A value outside them is taken as the nearest bound, and the result's first
  line says so (`intervalMs 900000 is above the cap; taken as 600000`), since
  the command goes on either way: models asked for fifteen minutes for a
  test suite six times in one conversation, and a refusal only cost them a
  call each time. A fraction is refused, not rounded.
- `shell` takes a `description`, the call's title for the user, and requires
  it ([Tool descriptions](rendering.md#tool-descriptions)).

### Running shell tools

- The shell tools reach the conversation's current Host through the
  conversation's host access
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
  Their jobs run in the runner's shell
  ([Shell jobs](../execution/runner.md#shell-jobs)).
- Every `shell` call starts a job of its own in its node's working
  directory on the primary Host ([Working directory](subagents.md#working-directory)),
  the conversation's for the root, the directory the context block names
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
  Nothing of an earlier command carries over: not its directory, its
  variables or its functions. For example, after a script that runs
  `cd browse && npm test`, the next call starts in the working directory
  again, not in `browse`. Where a call starts is therefore never a guess: a
  carried directory made a model's next `cd ./browse` fail inside `browse`,
  and models began every script with `cd <workspace>` because the context
  block's directory stopped being true after the first `cd`.
- Each node keeps one shell environment per Host it has used, which holds
  its commands' handles. Concurrent calls for one Host create one
  environment.
- A handle belongs to its environment. A `commandId` of another Host's
  environment is refused with `Command "<id>" belongs to a different Host`,
  and a handle that two environments claim is refused as not unique.
- The repeat guard counts identical scripts. In one environment, a `shell` call
  of the same script within 60 seconds of the previous one is allowed six times
  in a row. The seventh and later identical calls do not run: the result is an
  error that asks the model to inspect the previous output, and its view is
  `repeated_shell` with the script and the count. Another script resets
  the count.

### The window

For example, the model runs the test suite, and twenty seconds in a subagent
reports that the failing test is a flaky one. The window ends at once: the
call returns the running command's handle and its output so far, the agent
message enters the transcript, and the model can stop the command and run it
again without that test. Without that, the message would wait for the rest of
the minute.

A `shell` call watches its command until the first of:

- the command ends;
- a minute passes, or its interval when that is shorter, or, for a
  command whose interval is `null`, its output has been quiet for 2 seconds
  or 30 seconds have passed
  ([Command reports](#command-reports));
- an agent message or a command report arrives, which joins the turn at its
  next boundary ([Input](runtime.md#input));
- the user sends a steer or a queued message now, and the result says so
  ([Send now](runtime.md#send-now)).

Ending the window never stops the command: it keeps running, and the result
carries its handle, as when the time passes. A human steer ends no window: by
steering rather than sending now, the user chose to wait for the running call.
A message sent to the queue does not join the turn, so it ends no window
either.

### The `demi shell` commands

The runtime's own command group works on the conversation's commands. Each
is an `rpc` command, handled in the backend from any of the conversation's
shells, so a script combines them freely: `demi shell input 17 <<'EOF'`
answers a prompt and `demi shell status 17` in the same script shows what
the command did with the answer.

| Command | What it does |
| --- | --- |
| `demi shell status <commandId>…` | Prints each command's status, its exit code once it has ended, how long it has run and printed nothing, and its output since the node's last look at it, as a result shows it ([Results and previews](#results-and-previews)), also of a command that was stopped or lost: `status: stopped` or `status: lost: <reason>`, then the output; it shows the media's lines and attaches none ([What a result attaches](#what-a-result-attaches)). A command whose Host's connection is away, looked at from another Host, says so: `status: running, as far as Demi knows; its Host has been unreachable for 40 seconds, and its runner keeps it running`. `--wait <duration>` waits up to that long for the commands to end first; the `shell` call that runs it watches as any call does. `--interval <duration>` or `--resident` changes how the node hears from the commands from then on ([Command reports](#command-reports)). |
| `demi shell input <commandId>` | Writes its stdin to the command's input, such as an answer to a prompt, and prints nothing; input for a command that is not running fails. A command that still acquires its Host, as while a Cloud wakes, takes the input once it starts. |
| `demi shell output <commandId>` | Prints the command's whole output ([The whole output](#the-whole-output)). |
| `demi shell stop <commandId>…` | Stops each command, below. |

`demi shell stop <commandId>…` stops each running command of the conversation it names,
whichever agent ran it, and waits until it has ended: it prints
`[command 17 stopped]`, and the command's next status shows it
`stopped` with its last output. A command that had already ended prints
`[command 17 had already ended]` and also succeeds, so stopping is safe to
repeat; a number that names no command of the conversation fails with
`demi shell stop: 17: no such command in this conversation`, while the
others it names are still stopped. It stops the command
as the page's stop does,
through the command's shell environment: `TERM` first, so the command and
every program it started can end cleanly, and `KILL` for what has not
ended after 5 seconds
([Cancellation and completion](../execution/runner.md#cancellation-and-completion)).
`status` and `stop` read any command of the conversation, whichever agent
ran it; the place a status moves is the calling node's own.

### Results and previews

For example, `npm test` prints 4,720 lines, 212,345 bytes, and exits with
status 1 within its call's window. Its result shows the first and the last
lines of the output, and between them one line that says which lines it
leaves out and how to read them:

```text
status: exited
exitCode: 1
commandId: 17
output:
<lines 1-158>
[... lines 159-4562 not shown (196545 bytes); read them: demi shell output 17 --lines 159-4562 ...]
<lines 4563-4720>
```

- The model keeps its own place in each command's output: a result shows the
  output since the model's last look at the command. What the pages are sent
  never moves it ([Live output](#live-output)). For example, a page shows a
  running command's new output; the model's next `demi shell status` still shows
  all of it.
- A result gives the command's status, its exit code once it has exited, its
  `commandId`, its timings while it runs, the output, and a hint
  for the next step while it runs or once it was stopped: that the node
  will hear from the command as its interval says, and that
  `demi shell status`, `input` and `stop` look at it, answer it and stop it.
  A command whose script has ended but whose background tasks still run
  says so, naming them, since a model that started `nohup server &` and saw
  its command still running took it for a hang:
  `the script has ended; its background task "PORT=18948 bun src/server.ts"
  keeps the command running, and stopping the command stops it`.
- A result whose command exited with a status other than 0, or was stopped
  or lost, is marked as an error to the provider (`is_error` on Anthropic's
  API), as Claude Code marks a failed command, so the model weighs it as a
  failure; its text is the same.
- A result's `idleMs` counts from the last time the command's output grew,
  also beyond the first 8 KiB of a stream: the runner reports such growth
  within 2 seconds even while no page follows the command
  ([Pipes and output](../execution/runner.md#pipes-and-output)). For example,
  a build that prints its 100th KiB of log lines a second ago shows an
  `idleMs` below 3,000, so the model does not take it for a hung one.
- The model need not cut a command's output itself: the result is already
  bounded and points to the rest. The tool rules say so, since piping into
  `head` or `tail`, which models did in half of their scripts, hides the
  command's exit status behind the pipe's, hides a long command's output
  until it ends, and turns a returned image into a fragment of its bytes.
- The output is the merged stdout and stderr since the model's last look, in
  the order the runner read them. It is shown whole when the whole result fits
  the replay bound of 16,000 characters
  ([Text bounds](compaction.md#text-bounds)). Otherwise the result shows as
  many whole lines from the start and from the end as fit in half each of what
  the bound leaves after the result's other lines, the line between them
  included, and that line names the lines left out, their bytes, and the
  command that prints them ([The whole output](#the-whole-output)). A single
  line too long for its half is shown in part, and the line between names the
  characters left out and how to read them a part at a time:
  `[... characters 7901-40311 of line 1 not shown; read them: demi shell output 17 --raw | sed -n 1p | cut -c 7901-19900 ...]`.
  So a result is cut once, where it is made, and replay sends it unchanged.
  The rule is the same for every model. Characters are Unicode scalar values
  ([Token estimates](compaction.md#token-estimates)).
- While a command runs, the backend holds the first 8 KiB of each of its
  streams and, once a stream has gone beyond them, its newest 8 KiB, as the
  runner sent them within the last 2 seconds; what lies between stays on the
  Host ([Pipes and output](../execution/runner.md#pipes-and-output)). The
  result of a running command shows its output since the model's last look
  within the first 8 KiB, then, for each stream that has gone beyond them, a
  line that counts the bytes left out and the stream's newest whole lines,
  each part within half of the bound: for a dev server that has logged for an
  hour, its start, `[... 1040384 bytes of stdout not shown; its newest lines follow ...]`
  and its last requests. The model's place moves only within the first 8 KiB,
  so the next look shows the newest lines as they are then.
- The model's place moves past the whole lines a result covers, the lines
  it leaves out included. An unterminated line is shown at once, and the
  next look shows it again from its start, as far as it has grown: a command
  that prints `rea`, then `dy` and a newline, shows `rea`, then `ready`.
  Once the command ends, its last line counts as whole even without a
  newline.
- The handle serves `demi shell status`, `input` and `stop` while the
  command runs. A result that reports the command's end releases it;
  `demi shell output` goes on reading the output by its `commandId`.
- A result that reports a command's end attaches the media it viewed with
  `demi file view` ([Media the model views](#media-the-model-views)).

### Media the model views

The model looks at an image, a video or a PDF with `demi file view`, the one
command that shows the model a file. For example, it checks the sign-in page
and two frames of a recording in one call:

```text
$ demi browser screenshot t1 | demi file view
Screenshot of t1
Image: 1280 × 720 px, one pixel per CSS pixel
Viewport: 1280 × 720 CSS px, device pixel ratio 2, web
[image 1: image/png, 1280 × 720 px, 412000 bytes]
$ ffmpeg -loglevel error -i demo.mp4 -vf fps=1/5 frame-%02d.png && demi file view frame-01.png frame-02.png
[image 2: image/png, 1920 × 1080 px, 801233 bytes]
[image 3: image/png, 1920 × 1080 px, 799410 bytes]
```

`demi file view` reads each file whole before it prints its line, so a
picture that does not decode whole, such as one a writer cut short, fails
the command at once, `demi file view: fox.png: the image is cut short`,
and is never attached; a line it prints is a picture the model will see.
The result shows this output and attaches the three images after it, in
that order; the page shows them under the call's row
([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
In this section the script that a `shell` call runs is the **job**, which the
`commandId` names. A **medium** is one image, video or PDF document that the
job showed the model.

#### What `demi file view` shows

- **The command hands the medium over, not its stdout.** `demi file view
  [<path>...]` reads each path, or its stdin when it names no path or names
  `-`, and gives each one to its job as one medium: in a pipeline, a command
  substitution, a loop or a background task alike. It writes nothing to its
  stdout, and the rest of the script never sees the bytes. Stdin that is the
  job's own input, which `demi shell input` feeds
  ([A job's own input](../execution/runner.md#a-jobs-own-input)),
  is not read: with no path, the command fails with
  `demi file view: no file named, and stdin is the job's input; name a file or pipe one in`.
- **Lines in the output.** The runner numbers the job's media from 1 in the
  order they reach it and writes each one's line into the job's output as it
  arrives, after the output before it: `[image 1: image/png, 1280 × 720 px,
  412000 bytes]`, `[video 2: video/mp4, 12.4 s, 1920 × 1080 px, 8598311
  bytes]` (the duration and size when the container's header gives them),
  `[document 3: report.pdf, application/pdf, 182044 bytes]`. The word is the
  medium's kind; the number is the job's. A document carries its file's
  name to the model, which a provider's document part takes as its title;
  one from stdin is named `document-3.pdf`. The handler of `demi file view`
  reads these facts from the medium's header, once, and they travel with the
  medium; the runner writes the line only after it has read what the job's
  output already holds, so a line never comes before output written before
  the command.
- **What it takes.** Bytes, at most 16 MiB, of a media type the conversation's
  model reads in a tool result: a type of the model-media table
  ([Accepted attachment types](../providers/models.md#accepted-attachment-types))
  that the model accepts and its provider carries in a tool result
  ([Media in tool results](../providers/providers.md#media-in-tool-results)).
  The job carries that list from its start, as the backend gives it for the
  node's model, with the model's name for the refusal line. A job that
  `demi host shell` starts on another Host carries none, since its output
  is relayed into the invoking command and no result reports its end: there
  the command fails with
  `demi file view: shot.png: a job on another Host shows the model nothing; pipe its bytes into demi file view in your own script`,
  as `demi host shell --host laptop 'cat shot.png' | demi file view` does. A path that fails gets a line on stderr; the command goes on
  with the next one and exits with status 1:
  - `demi file view: notes.txt: a text file; read it with cat notes.txt`
  - `demi file view: data.bin: not an image, a video or a PDF (48213 bytes)`
  - `demi file view: report.pdf: this conversation's model, deepseek-v4.1-flash, does not read application/pdf in a tool result`,
    or, when which types the model reads is unknown,
    `demi file view: shot.png: it is not known which files this conversation's model, deepseek-v4.1-flash, reads; its provider entry can name them`
  - `demi file view: capture.mov: 23.1 MiB; a medium is at most 16 MiB`
  - `demi file view: shot.png: No such file or directory`, the system's own
    words ([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)).

  Stdin is named `stdin` in these lines.
- **Every other command writes bytes.** A program that makes an image writes
  its bytes as any program does, so `> shot.png` saves them and
  `| demi file view` shows them; `demi browser screenshot` does the same
  ([Images and large outputs](../browser/browser.md#images-and-large-outputs)).
  Bytes are binary when their first 8 KiB hold a NUL byte or they read as
  a media type Demi recognises, as git and GNU diff decide; any other bytes
  are text, and a byte that is no UTF-8 shows as U+FFFD, as a terminal
  shows it. One rule decides it for a job's stdout and for edit tracking's
  copies ([Edit tracking](../execution/edit-tracking.md)), so a stray byte,
  such as a raw line `grep` printed from a git object, costs one character
  rather than hiding the whole output, as it did in a product check. A
  job's stdout that is binary is never shown to the model: the output
  shows it as one line, `<binary stdout: 412000 bytes>`, and the result's
  line says what to do instead:
  `[binary stdout, 412000 bytes: not shown; to look at an image, a video or a PDF, pipe it into demi file view; to keep it, redirect it to a file]`.

| The script | What the model gets |
| --- | --- |
| `demi file view a.png b.png` | Two images, in that order |
| `demi browser screenshot t1 \| demi file view` | The screenshot, with its description from the screenshot's stderr |
| `demi browser screenshot t1 > shot.png` | Nothing of the image: `shot.png` holds the PNG |
| `demi browser screenshot t1 \| convert - -resize 50% png:- \| demi file view` | The half-size image alone |
| `for t in t1 t2 t3; do demi browser screenshot "$t" \| demi file view; done` | Three images, in the loop's order |
| `demi file view a.png & demi file view b.png & wait` | Two images, numbered in the order they reached the runner |
| `x=$(demi file view a.png)` | The image; `x` is empty |
| `cat shot.png` | The line `<binary stdout: 412000 bytes>` and the result's line saying to pipe it into `demi file view` |
| `demi host shell --host laptop 'cat shot.png' \| demi file view` | The laptop's image: the relayed bytes are this job's stdin to `demi file view` |

A medium goes to the model through the command that was asked to show it,
never through where its bytes happen to flow, so a file the script only
copies, pipes or saves never reaches the model, and one it views always
does, whatever the shell did with the command's stdout.

#### What a result attaches

Media reach the model only in the result that reports the job's end: the
`shell` result when the job ends within its window, otherwise its end
report ([Command reports](#command-reports)). A result that shows the job
running attaches none, so each medium is attached once and a job's media
arrive together, in their order. A `demi shell status` that shows the end
shows the media's lines and attaches nothing, since it is a job of its own:
the end report then still comes, carrying the media alone, though the look
showed the end.

The result attaches each of its media, in order, while these rules hold:

| Rule | Why |
| --- | --- |
| The model reads the medium's type in a tool result | A request carries nothing the model does not read; the model can change while the job runs |
| An image fits as [Images in the transcript](transcript.md#images-in-the-transcript) fits it, and enters fitted; a video or a document is at most 16 MiB | Every request sends the same bytes, which every provider accepts |
| It is one of the result's first 20 attached media | One step's result stays a small part of a request: the fewest images a documented vendor takes in one request is 100 ([Request limits](../providers/models.md#request-limits)) |
| The base64 of the result's attached media, this one included, takes at most half of the model's request body limit ([Request limits](../providers/models.md#request-limits)) | A request still has room for the history around it. A medium that breaks this rule is skipped, and a later, smaller one may still be attached |

After the output, the result gives one line for each medium it did not
attach, and for each it attached in another form than it came, with what
happened; the original is the file the model viewed, so a line needs no way
to read it:

```text
[image 2: not attached: the model does not read image/webp]
[image 3: attached as image/jpeg of 2000 × 1000 px, fitted from image/png of 3000 × 1500 px]
[image 21: not attached: a result attaches at most 20 media; view it in another call]
[image 4: not attached: lost with the Host's connection]
```

#### Bounds and cut output

- A medium is at most 16 MiB, and a job keeps at most 32 media and 64 MiB of
  them. The runner keeps none beyond these: in that medium's place the
  job's output reads
  `[image 33: not kept: a job keeps at most 32 media and 64 MiB]`, and
  `demi file view` goes on. A job's media therefore take at most 64 MiB of
  its Host's disk.
- Media are kept apart from the output, so neither the output's 16 MiB bound
  nor a result's cut touches them. A build that prints 20 MiB of log and then
  views a screenshot attaches the screenshot, even when its line lies in the
  part of the output left out.

#### Where media are kept

The runner keeps a job's media in the job's directory as they arrive and
tells the backend of each one
([Pipes and output](../execution/runner.md#pipes-and-output)). When the job
ends, the backend reads each medium the result attaches, fits it, and stores
it as a blob in the conversation owner's namespace before the result reports
the end; the result holds it by reference, as every tool medium
([Media](transcript.md#media)). A medium the result does not attach is not stored: the
model views its file again when it needs it.

### The whole output

A result shows the first and the last lines of a long output;
`demi shell output` reads all of it, a page at a time, the way a file is
read. For example, after the `npm test` above, the model finds the failures
and reads around one:

```text
$ demi shell output 17 --raw | grep -n 'FAIL'
2301:FAIL src/parse.test.ts
$ demi shell output 17 --lines 2295-2310
[command 17: lines 2295-2310 of 4720, stdout and stderr]
  2295	  ✓ parses an empty file (3 ms)
  ...
  2301	FAIL src/parse.test.ts
  ...
  2310	    at Object.<anonymous> (src/parse.test.ts:88:5)
```

`demi shell output <commandId>` is an `rpc` command
([Command declarations and execution](../execution/commands.md)): it runs in
the backend, from any shell of the conversation, the root's or a subagent's,
on any of the conversation's Hosts. Everything it prints is sized for the
model:

- **A page.** It prints numbered lines, as `cat -n` does: from the first line,
  or the lines `--lines <from>-<to>` names, as many whole lines as fit in
  12,000 characters, so its own result is never cut. A first line in brackets
  says what the page holds; when lines follow, a last line gives the command
  for the next page: `[next: demi shell output 17 --lines 2311-4720]`.
- **The newest lines.** `--tail <n>` prints the last `n` lines, or as many of
  them as fit; of a running command, the newest so far.
- **One stream.** `--stdout` or `--stderr` takes that stream alone, with line
  numbers of its own. Without either, the page holds both, merged in the
  order the runner read them, as the result showed them.
- **The bytes.** `--raw` prints the output as it is: unnumbered, unpaged,
  byte for byte, for pipes and files. Its lines are the pages' lines, so
  `grep -n` on it gives the numbers `--lines` takes, and
  `demi shell output 17 --raw --stdout > shot.png` saves a binary stdout. Printed into a result as it is, raw output is cut as
  any output is. Searching is the standard tools' work on `--raw`; the command
  has no search of its own.
- **A long line.** A page shows a line of more than 2,000 characters as its
  first 2,000 and a note after it:
  `[line 2301 is 48212 characters; whole: demi shell output 17 --raw | sed -n 2301p]`,
  so one line cannot fill a page.
- **Which commands.** Any command of the conversation, whichever agent ran it.
  A command of another conversation is unknown to it.
- **A command that runs.** It reads what the command's Host has kept so far,
  through the conversation's host access
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
- **A command that ended.** It reads the output the backend stored, which
  stays ([Retention](../backend/storage.md#retention)).
- **What is kept.** A command's output up to 16 MiB, all of it; beyond that,
  its first 8 MiB and its last 8 MiB. Lines are numbered as kept, and the
  merged pages show an unnumbered line where the rest was left out,
  `[... 734003200 bytes left out ...]`; `--raw` writes that line to stderr, so
  the bytes stay as they were. A command that prints without end, such as
  `yes`, therefore keeps at most 16 MiB, on its Host and in the backend alike.
- **A binary stdout** shows in the merged pages as the line the result shows,
  `<binary stdout: 412000 bytes>`; `--stdout` without `--raw` answers that the
  stream is binary and names `--raw`.
- **Failures** go to stderr with exit status 1:
  - `demi shell output: no command 17 in this conversation`;
  - `demi shell output: the output of 17 was not stored: <reason>`, when the
    backend could not store it;
  - `demi shell output: lines 5000-5100 are past the end: the output has 4720 lines`.
- **A reader that stops early**, such as `| head -n 20`, ends the command
  quietly ([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)).

Where the output is kept:

- While the command runs, on its Host, in the job's directory, within the same
  16 MiB ([Pipes and output](../execution/runner.md#pipes-and-output)).
- When the command ends, the backend stores its output before any result
  reports the end. When each stream stayed within its first 8 KiB, which is
  most commands, and every byte of them reached the backend, the backend
  already holds the whole output, in the order the runner read it, and stores
  that; otherwise, as when bytes printed while the Host's connection was
  lost were not read again before the end, it reads the Host's kept output
  once, as it reads the command's edit copies
  ([Edit tracking](../execution/edit-tracking.md)). The output becomes a blob
  in the conversation owner's namespace, and the conversation records it by
  its command ([Command outputs](../backend/storage.md#command-outputs)). The
  Host then removes the job's directory. The result that reports the end is
  made from the stored output, so the lines it names are the pages' lines.
- A command that ended because its Host's connection was lost keeps what the
  backend received of each stream from its start, up to the first byte it
  missed, followed by the line
  `[... 1048576 bytes lost with the Host's connection ...]` when there was
  more. So does a command whose kept output the backend could not read from
  its Host, with the line
  `[... 1048576 bytes not read from the Host: <reason> ...]`.
- The stored output is never part of a request, so storing it or removing it
  changes no request ([Prompt cache](../providers/providers.md#prompt-cache)).

### Live output

For example, the model runs `npm test` with a two-minute window while the
conversation is open in tabs A and B. Both tabs show the test output under the
call as the runner reads it. The window ends while the tests still run: the
call returns the command's handle to the model, and both tabs count the
command in the dock's Running chip, whose panel keeps showing its output as it
comes, although the model no longer looks at it. Tab C, opened now, shows the
end of the output at once and then follows as A and B do. When the tests end,
all three show the end.

```text
the job's stdout and stderr
  |
  v
runner: job_output, each with its stream and offset
  |       the first 8 KiB of each stream, always
  |       beyond them, while followed: the newest bytes,
  |       at most 16 KiB per stream every 250 ms;
  |       while not: the newest 8 KiB, at most every 2 s
  v
backend: the command's record
  |       the model's place and text
  |       the pages' view: its last 4,096 characters and their count
  v
the tree: shell_output to every attached page,
          new output at most every 250 ms, an end at once
```

- **The pages' view.** A command keeps one view of its output for the pages,
  apart from the model's place: its output in the order it reached the
  backend, with a note where the runner left some out. The pages are sent the
  view's last 4,096 characters (`tail`) and how many characters the view has
  held since the command started (`chars`); characters are Unicode scalar
  values. The view is the same for every page, whatever any page or the model
  read, and no read changes it. When the command ends, the view gets what the
  end adds: when the backend had not received all of the output, the whole
  output it stores ([The whole output](#the-whole-output)), whose end the
  pages then show anew; why the command could not run; or a binary stdout's
  description.
- **Which commands.** Every command of the tree, the root's and each live
  subagent's, several at once, on a Cloud and on a paired device alike: both
  are Hosts behind a runner.
- **While a page is attached.** While at least one connection is attached to
  the tree ([Connections and the live tree](rendering.md#connections-and-the-live-tree)),
  the backend follows every running command of the tree: the runner then sends
  each stream's output beyond its first 8 KiB as it comes. A command's start and
  every change of its view reach every attached connection as `shell_output`:
  the start and new output at most every 250 ms, the tree's changed commands
  together, and the command's end at once. Output that comes after a quiet
  quarter of a second goes at once. A command therefore sends a page at most
  four frames a second however much it prints, and cannot fill an outbox
  ([Order and delivery](rendering.md#order-and-delivery)) by itself.
- **A page that attaches** receives each live command's view in its
  handshake, then every change as the other pages do. A live command is one
  that runs in a node's shells, or that the node's transcript last shows
  running and its shells still hold. When no page was attached before, the
  backend was not following, so the handshake shows what the backend holds;
  the runner's newest output follows as soon as the runner starts following.
- **No page attached.** When the last connection detaches, the backend stops
  following and sends nothing; the runner sends each stream's first 8 KiB
  again and, beyond them, the stream's newest 8 KiB every 2 seconds, which the
  model's view shows and its idle time counts from
  ([Results and previews](#results-and-previews)). The next
  connection to attach starts from its handshake.
- **The end.** A command's last frame shows its end: its exit, a stop (a
  page's stop, `demi shell stop`, a Stop of the action that
  started it), or the end of its node's shells: the abort or close of its
  subagent, or the tree's interruption for a Host transition. The tree's
  disposal ends no command: its commands run on and report when the tree is
  live again ([Dispose and restore](runtime.md#dispose-and-restore)). No frame of the
  command follows its end.

What keeps the output coming, and where each part is released:

| Part | Lives in | Released |
| --- | --- | --- |
| The runner's messages beyond a stream's first 8 KiB, with their timer | The job's task on the runner | With the job; the timer runs only while a message waits |
| The backend's following of a job: it watches whether a page is attached and tells the runner | The job's task in the node's shell environment | With the job |
| The commands whose new output is not sent yet | The tree | Emptied when sent, when a command ends, and when the last connection detaches |
| The task that sends them | The tree; it has a timer only while commands wait | With the tree |
| The reports of new output and ends | The tree's feed, which each shell environment of each node holds without keeping the tree alive | With the environment |

### Dispatch and failures

- Before the first tool of a round runs, the session saves its checkpoint, so
  every call it is about to run is stored as executing. A process that dies
  during a tool leaves the call executing, and restore completes it as
  interrupted without running it again ([Dispose and restore](runtime.md#dispose-and-restore)).
- A round's calls run in steps, in the order the model requested them:
  consecutive `shell` calls are one step and start together; each other
  call is a step of its own. A step ends when each of its calls has returned,
  and the next starts then. Each result is recorded as its call returns, in
  its call's block, so the page shows a call's end and its changed files
  while the step's other calls run; the model still reads the results in the
  order of the calls. Waiting input is written after each step
  ([Input](runtime.md#input)). Since every call is `shell`, consecutive calls are one
  step: a response's calls all start together.
- Each `shell` call of a step is a job of its own
  ([Running shell tools](#running-shell-tools)), so the calls of a step never
  wait for one another.
- A Stop during a step ends the calls still running; a call that had
  already returned keeps its result.
- Each call takes its own lease of the conversation's file gate, so a step's
  calls never take a lease while holding another
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
- The model is told that the commands of one response run at the same time,
  so commands that depend on each other go in one script, joined with `&&`,
  or in separate responses. Models already treat the calls of one response
  as independent, and harnesses such as Claude Code run them together.
- A tool that fails completes its call as an error `Tool failed: <message>`.
- An action that fails before its calls ran, such as when the save before
  dispatch fails, completes them as aborted, with that failure as the
  reason, as a stop does, so the next request replays no call without a
  result.
- A tool never reaches into its session. It returns its result.

### Interrupted calls

For example, the backend restarts for a configuration change while the
model watches command 254, a test suite. The model's next request shows the
call ended:

```text
Tool call interrupted: the backend shut down while this call watched command 254, which keeps running; look at it with demi shell status 254.
```

A call that something other than its tool ends completes as an error that
says what ended it and what became of its command, since a model told only
`Tool call aborted: shell_exec` spent calls on `ps` and `journalctl` to find
out:

| What ended the call | Its result |
| --- | --- |
| The user's Stop | `Tool call aborted: the user stopped the turn; command 254 was stopped.` |
| A hold of the conversation, such as a Cloud reset or a deletion | `Tool call aborted: <the hold, such as "the Cloud is being reset">; command 254 was stopped.` |
| The backend's shutdown | `Tool call interrupted: the backend shut down while this call watched command 254, which keeps running; look at it with demi shell status 254.` |
| The backend's crash, found at restore | `Tool call interrupted: the backend stopped before its result was recorded; command 254 keeps running if its Host kept it.` |
| An action that failed before its calls ran | `Tool call aborted: <the failure>.` |

A call that watched no command ends the line after the reason. A call that
watched one follows the line with the command's output since the model's
last look, as a result shows it, so the model sees how far the command got
without looking again: after a Stop in a product check, the model's next
step was `demi shell status` only to read `ok group 1`. A command an
interrupted call leaves running, after a shutdown or a crash, reports as any
command whose call returned does, at the interval it was started with, which
its record keeps, and its end, or, lost, as a resident or a numbered command
by that interval ([Command reports](#command-reports)).

### Lost commands

A command whose runner lost it, which the backend learns when the runner
connects again ([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)),
ends as lost with its reason, and the node that ran it is told as of any
other end ([Command reports](#command-reports)), with one exception: the loss
of a resident command, one whose interval was `null` when it was lost, opens
no turn. Its report joins the node's running turn at its next boundary, or
waits, as a pending report, for the node's next turn and is written before
that turn's first request. For example, a conversation left idle with its
dev server and emulator running is no longer woken, and charged a request,
by every upgrade that replaces its runner; the next time the user writes to
it, the model reads first that both were lost. A lost command started with a
number still wakes its node, since the model may be waiting for it:
`Command 437 (bun run dev) was lost: Demi was upgraded and the Host's runner
replaced itself. Start it again if it is still needed.` A conversation
browser tab that its browser lost reaches the node the same way:
`Tab t1 was closed: the browser ended with the Host's runner.` The command's
handle answers `demi shell status` and `demi shell output` with the same reason.
Before, a dev server lost with a two-second connection drop went unnoticed
for three hours, until the agent's next `demi shell stop` said it had
already ended.
