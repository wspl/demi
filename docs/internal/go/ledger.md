# Behavior ledger — Gate 0 baseline (draft, F5)

Source: `/home/user/demi-base` at `6e043eb1`. TS tests found under
`packages/*/src/**/__tests__/*.test.ts` and `*.test.ts` (271 files found by
that glob; behaviors below cover the 179 files in scope — frontend packages
and frontend-side libraries are excluded, see the note at the end). Rust
tests found under `crates/*/tests/*.rs` and `#[cfg(test)]` / `#[test]` /
`#[tokio::test]` modules in `crates/*/src/**` (64 files, ~241 tests).

Method: test names in this codebase are already written as behavior
statements, so most rows below reuse the test's own wording, merging
sibling tests of the same behavior into one row. Rows marked `internal`
test an implementation step, not an observable boundary, and are not
ported as separate behaviors (the boundary behavior that depends on them
is ported instead).

Two discrepancies between `docs/internal/go/plan.md` and the code, resolved
here rather than guessed past (flagged again in the handback message):

- Plan lists `pipes.rs` as an S1 (shell) test file. The only `pipes.rs` at
  that path (`crates/runner/tests/pipes.rs`) exercises
  `demi_runner::pipes::PipeClient`, the outbound network pipe client that
  plan's own S2 row owns (`crates/runner/src/{...,pipes,...}`). Assigned to
  S2 below.
- `crates/runner/tests/utilities_cat.rs` and `utilities_catalog.rs` are not
  named in plan's S1 test list but exercise `shell::utilities` dispatch
  (the exec handler S1 owns), not any one utility's behavior. Assigned to
  S1, noted as depending on the T1a–T2c utilities existing.

---

## S1

Owns: shell core (fork of mvdan/sh; job scope, completion, cancellation,
exit status; open handler edit recording; exec handler routing; login
profiles/HOME; background jobs, here-docs, process substitution).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | A builtin pipeline emits output before the input side reaches EOF (no deadlock waiting for the whole input) | `runner/tests/shell.rs`: `builtin_pipeline_emits_before_input_eof` | behavior |
| 2 | Redirects, shell functions, subshells, `cd`, and fresh per-job state (env, cwd) behave as one coherent shell session | `runner/tests/shell.rs`: `shell_handles_redirects_functions_subshell_cwd_and_fresh_state` | behavior |
| 3 | `tee` and `od` consume/produce through pipeline streams, not temp files | `runner/tests/shell.rs`: `tee_and_od_use_pipeline_streams` | behavior |
| 4 | A job joins its background tasks before exiting, and reports the foreground command's exit status, not the background one's | `runner/tests/shell.rs`: `job_joins_background_tasks_and_preserves_foreground_exit_status`; `runner/tests/tasks.rs`: `jobs_share_the_runner_process_and_cancellation_is_isolated` | behavior |
| 5 | Login profiles apply per job (each job gets its own login env) without replacing the job's already-owned context or cwd | `runner/tests/shell.rs`: `login_profiles_apply_per_job_without_replacing_owned_context_or_cwd` | behavior |
| 6 | Windows drive-letter paths resolve for `cd`, redirection, utilities and executables | `runner/tests/shell.rs`: `windows_drive_paths_work_for_cd_redirection_utilities_and_executables` | internal (non-Linux; plan restricts to Linux x86_64 for now — recorded, not ported until another target lands) |
| 7 | A job's full output is kept for the record, but live/status views only ever send a head and a tail slice, not the whole stream | `runner/tests/tasks.rs`: `shell_job_keeps_full_logs_but_only_sends_head_and_tail_views` | behavior |
| 8 | Functions and compound pipelines correctly drain large output and here-documents without truncation or deadlock | `runner/tests/tasks.rs`: `functions_and_compound_pipelines_drain_large_output_and_here_documents` | behavior |
| 9 | Cancellation terminates a blocking native builtin, reports the exit as the requesting signal, and reaps external programs the builtin started | `runner/tests/tasks.rs`: `cancellation_terminates_a_blocking_native_builtin`, `shell_cancellation_reports_the_requesting_signal`, `cancellation_reaps_external_programs_started_by_native_utilities` | behavior |
| 10 | Runner shutdown does not block waiting for a consumer that stopped reading a job's output | `runner/tests/tasks.rs`: `shutdown_does_not_wait_for_a_blocked_output_consumer` | behavior |
| 11 | Job completion preserves the output of process substitution (`<(...)`) | `runner/tests/tasks.rs`: `job_completion_preserves_process_substitution_output` | behavior |
| 12 | Edit tracking: redirections, descriptor writes and utility writes each record the file's actual resulting contents as one edit; output an external program writes through a redirect is forwarded to the recorder too; once a job has captured a file's "after" state, a different concurrent job cannot change what was captured | `runner/tests/edit_tracking.rs`: `redirections_descriptors_and_utilities_record_actual_contents`, `redirected_external_output_is_forwarded_through_the_recorder`, `another_job_cannot_change_an_already_captured_after_side` | behavior |
| 13 | Under load, filesystem and working-tree requests queue and wait rather than fail, and native/backend calls are never turned away | `runner/tests/load.rs`: `native_calls_are_never_turned_away`, `backend_commands_are_never_turned_away`, `filesystem_requests_wait_instead_of_failing`, `working_tree_requests_wait_instead_of_failing` | behavior |
| 14 | Running out of open file descriptors makes a job wait for one to free up instead of failing | `runner/tests/open_files.rs`: `running_out_of_open_files_waits_instead_of_failing` | behavior |
| 15 | Two uutils-style builtins can stream through OS pipes within one process; each utility invocation gets its own cwd and exit-status state, isolated from concurrent invocations | `runner/tests/utilities_cat.rs`: `two_uutils_builtins_stream_through_os_pipes_in_one_process`, `uutils_cwd_and_exit_state_are_per_invocation` | behavior |
| 16 | Every registered utility routes `--help` to the invocation's own output stream, not a shared one; filesystem, recursive, and search/edit/compare utilities resolve relative paths against the invocation's own directory, not a process-global cwd; `env` runs a child with the invocation's own environment and streams its output; concurrent external sorts (`sort`) each own their temp files and worker processes; symlink and directory-mode handling round-trips; `jq` and `ripgrep` see the invocation's own files, arguments and environment, honoring upstream option syntax | `runner/tests/utilities_catalog.rs`: `every_utility_routes_help_to_the_invocation_stream`, `filesystem_utilities_use_the_invocation_directory`, `env_executes_child_with_local_environment_and_streams`, `recursive_operations_stay_in_the_invocation_directory`, `concurrent_external_sorts_own_their_temporary_files_and_workers`, `relative_symlinks_and_explicit_directory_modes_are_preserved`, `search_edit_and_compare_utilities_keep_their_cli_and_local_paths`, `jq_uses_full_filters_files_arguments_and_invocation_environment`, `ripgrep_searches_and_filters_local_files_with_upstream_options` | behavior (this is the exec-handler contract every T1a–T2c utility must satisfy; depends on those WPs for the utilities themselves) |

Fixtures/goldens: none beyond in-test tempdirs; no golden files. Depends on
T1a–T2c utility implementations existing to exercise row 16 meaningfully.

---

## S2

Owns: runner connection and host operations (registration, pairing, claim,
wire codec vs. the old TS backend, filesystem/process ops, pipes, file
contents, Host log, management, command-alias mode, state).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The MessagePack wire codec decodes binary blobs and dates exactly as the TypeScript codec wrote them | `runner/src/connection/wire/tests.rs`: `decodes_binary_and_dates_written_by_the_typescript_codec`; TS side: `packages/runner-protocol/src/__tests__/protocol.test.ts`: "runner messages round-trip through the MessagePack wire" | behavior (wire contract, both ends must agree — TS test is the other half of this same behavior) |
| 2 | The wire codec rejects a value disguised as binary, unknown fields it does not declare, and trailing data after a message | `runner/src/connection/wire/tests.rs`: `rejects_array_disguised_as_binary_unknown_fields_and_trailing_data` | behavior |
| 3 | Generated wire validation checks optional fields and nested unions correctly | `runner/src/connection/wire/tests.rs`: `generated_wire_checks_optional_fields_and_nested_unions` | internal (schema-generation correctness; covered end-to-end by row 1–2) |
| 4 | Reading the Host log bounds a request to its declared limit, and log lines carry real timestamps | `runner/src/connection/wire/tests.rs`: `log_read_bounds_its_limit_and_log_lines_carries_times_as_timestamps` | behavior |
| 5 | Reading the Host log returns the newest lines first, then continues correctly from a cursor on the next read | `runner/src/host_log.rs`: `reads_the_newest_lines_then_continues_from_the_cursor` | behavior |
| 6 | An empty log answers with no lines and keeps the cursor unchanged | `runner/src/host_log.rs`: `an_empty_log_answers_no_lines_and_keeps_the_cursor` | behavior |
| 7 | When multiple log sources exist, the reader keeps one and advances the cursor past the others | `runner/src/host_log.rs`: `keeps_one_source_and_moves_the_cursor_past_the_others` | behavior |
| 8 | Log rotation and process restart replace the older file; a cursor taken before rotation/restart survives and keeps working | `runner/src/host_log.rs`: `replaces_the_older_file_and_a_cursor_survives_rotation_and_restart` | behavior |
| 9 | A cursor pointing at a log file that no longer exists restarts at the oldest available line rather than failing | `runner/src/host_log.rs`: `a_cursor_from_a_log_that_is_gone_starts_at_the_oldest_line` | behavior |
| 10 | A read that lands mid-line skips the partial half-line and ends cleanly before the next full line | `runner/src/host_log.rs`: `skips_half_a_line_and_ends_it_before_the_next` | behavior |
| 11 | A log page stops at its byte budget, and the returned cursor lets the next page continue correctly | `runner/src/host_log.rs`: `a_page_stops_at_its_byte_budget_and_the_cursor_continues` | behavior |
| 12 | Reads correctly reassemble chunks that do not end on a newline boundary | `runner/src/host_log.rs`: `splits_chunks_that_do_not_end_at_a_newline` | internal (I/O chunking detail underlying rows 5–11) |
| 13 | Lines queued for the log reach disk by the time the log is closed, and survive the log being reopened | `runner/src/host_log.rs`: `queued_lines_reach_the_files_by_close_and_survive_reopening` | behavior |
| 14 | Volume growth reservations use a fractional floor and cap growth for small volumes | `runner/src/volumes.rs`: `growth_reserve_has_fraction_floor_and_small_volume_cap` | behavior |
| 15 | The backend URL builder preserves an explicit path and query string | `runner/tests/connection.rs`: `backend_url_preserves_explicit_path_and_query` | behavior |
| 16 | A typed request/response exchange completes correctly, and the far side closing the connection is observable | `runner/tests/connection.rs`: `typed_exchange_and_remote_close` | behavior |
| 17 | Closing a connection interrupts output that is stalled mid-stream instead of hanging | `runner/tests/connection.rs`: `close_interrupts_stalled_output` | behavior |
| 18 | A malformed inbound message fails the connection cleanly | `runner/tests/connection.rs`: `malformed_input_fails_connection` | behavior |
| 19 | The filesystem wire preserves binary content, dates, symlinks, and OS error codes across the wire | `runner/tests/fs.rs`: `filesystem_wire_preserves_binary_dates_links_and_error_codes` | behavior |
| 20 | A reply larger than the message size limit fails that request without corrupting the connection | `runner/tests/fs.rs`: `a_reply_over_the_message_limit_fails_its_request` | behavior |
| 21 | Filesystem requests and a kill signal remain servable while a job is running (the connection isn't blocked by the job) | `runner/tests/host.rs`: `filesystem_requests_and_kill_remain_available_during_job` | behavior |
| 22 | A job's environment is the union of the device-level request environment and the job's own owned context, with defined precedence | `runner/tests/host.rs`: `job_environment_combines_device_request_and_owned_context` | behavior |
| 23 | A raw (non-shell) spawn inherits the runner process's environment only when the caller explicitly asked for that | `runner/tests/host.rs`: `raw_spawn_inherits_environment_only_when_requested` | behavior |
| 24 | The private/local endpoint streams binary input on demand and joins calls that were cancelled mid-flight | `runner/tests/local.rs`: `private_endpoint_streams_binary_input_on_demand_and_joins_cancelled_calls` | behavior |
| 25 | A client waits for a runner that is merely busy, but does not wait for one that is gone | `runner/tests/local.rs`: `a_client_waits_for_a_busy_runner_but_not_for_a_gone_one` | behavior |
| 26 | Command-alias mode: a backend job invokes the same binary under its alias name, and draining releases the installation lock | `runner/tests/mode.rs`: `backend_job_invokes_same_binary_alias_and_drain_releases_installation` | behavior |
| 27 | A child process streams binary output before stdin reaches EOF, and is reaped on completion | `runner/tests/process.rs`: `child_streams_binary_before_stdin_eof_and_reaps` | behavior |
| 28 | Cancellation interrupts a process even while output is back-pressured | `runner/tests/process.rs`: `cancellation_interrupts_output_backpressure` | behavior |
| 29 | Cancellation kills the whole descendant process group, not just the direct child | `runner/tests/process.rs`: `cancellation_kills_descendant_process_group` | behavior |
| 30 | Spawn failure distinguishes "cwd does not exist" from "executable not found" in the reported error | `runner/tests/process.rs`: `spawn_failure_distinguishes_missing_cwd_from_missing_executable` | behavior |
| 31 | Outbound network pipes: a quiet upload with delayed headers, and a body that outlives the connect deadline, both still complete | `runner/tests/pipes.rs`: `quiet_uploads_delayed_headers_and_bodies_outlive_the_connect_deadline` (misfiled under S1 in plan.md; content is `demi_runner::pipes::PipeClient`, an S2 module — see note at top) | behavior |
| 32 | Explicit cancellation of a network pipe interrupts quiet input, and a redirect cannot change the request's origin (SSRF-style protection) | `runner/tests/pipes.rs`: `explicit_cancel_interrupts_quiet_input_and_urls_cannot_change_origin` | behavior |

Fixtures: none beyond in-process TCP listeners/tempdirs. No golden files.

---

## S3

Owns: native package execution (manifests, artifact cache, command-service
client), resident services, git status/diff (go-git), working tree/watch.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The TypeScript-generated manifest's `--help` text and CLI cases match what the Rust side produces for the same declared commands | `runner/src/commands/manifest/mod.rs`: `typescript_manifest_help_and_cli_cases_match_rust` | behavior |
| 2 | A manifest whose descriptors or content were altered after being verified is rejected | `runner/src/commands/manifest/mod.rs`: `altered_descriptors_and_manifest_content_are_rejected` | behavior |
| 3 | The tree-watch layer records what git-relevant state *could* change, without doing the (expensive) read itself | `runner/src/git.rs`: `the_watch_notes_what_can_change_what_git_lists` | internal (an optimization the black-box behaviors below cover) |
| 4 | Walking one path of a staged rename also picks up its paired other side | `runner/src/git.rs`: `a_walk_over_one_path_of_a_staged_rename_takes_in_the_other` | behavior |
| 5 | A directory outside any git repository correctly reports as not being one | `runner/tests/git.rs`: `a_directory_outside_any_repository_is_not_a_repository` | behavior |
| 6 | Changes are judged against `HEAD`, and reported with line-add/remove counts | `runner/tests/git.rs`: `changes_are_judged_against_head_with_line_counts` | behavior |
| 7 | Every changed path carries the same status letter `git status` would print for it | `runner/tests/git.rs`: `every_path_carries_the_letters_git_status_prints_for_it` | behavior |
| 8 | A root inside the work tree lists only its own subtree, with paths relative to that root | `runner/tests/git.rs`: `a_root_inside_the_work_tree_lists_its_subtree_with_relative_paths` | behavior |
| 9 | The change list stops at a configured file-count limit rather than growing unbounded | `runner/tests/git.rs`: `the_list_stops_at_the_file_limit` | behavior |
| 10 | Reading a committed file's contents reads the last commit, and refuses what it structurally cannot read (binary, missing) | `runner/tests/git.rs`: `show_reads_the_last_commit_and_refuses_what_it_lacks` | behavior |
| 11 | A cancelled git-status request answers "cancelled" rather than hanging or erroring differently | `runner/tests/git.rs`: `a_cancelled_request_answers_cancelled` | behavior |
| 12 | A live watch of git status: later requests follow the filesystem watch, and a new commit makes the diff start over from the new base | `runner/tests/git.rs`: `later_requests_follow_the_watch_and_a_commit_starts_over` | behavior |
| 13 | Watched requests honor `.gitignore`-style ignore rules and file mode changes | `runner/tests/git.rs`: `watched_requests_follow_ignore_rules_and_modes` | behavior |
| 14 | An ignore rule declared above the watched root still reaches into it | `runner/tests/git.rs`: `a_rule_above_the_root_reaches_under_it` | behavior |
| 15 | A staged rename stays reported as one logical entry across successive watched requests, not as a delete+add pair that flickers | `runner/tests/git.rs`: `a_staged_rename_stays_one_entry_across_watched_requests` | behavior |
| 16 | The git wire format carries both changes and error conditions | `runner/tests/git.rs`: `the_wire_carries_changes_and_errors` | behavior |
| 17 | A filesystem-watch event correctly reports what changed, and does not itself trigger a read of the changed content | `runner/src/tree_watch.rs`: `a_notify_event_reports_changes_and_not_reads` | behavior |
| 18 | fsevents-specific flags (macOS) are correctly classified as content, metadata, or "events were lost" | `runner/src/tree_watch.rs`: `fsevents_flags_report_content_metadata_or_loss` | internal (macOS-only backend; Linux uses inotify — recorded, not a Linux port requirement) |
| 19 | A cancelled artifact download waiter does not cancel the shared download for other waiters, and the cached artifact is verified (checksum) before being served | `runner/tests/artifact_cache.rs`: `cancelled_waiter_does_not_cancel_shared_download_and_cache_is_verified` | behavior |
| 20 | A downloaded artifact whose size does not match its manifest leaves no cache file and no stray temp download | `runner/tests/artifact_cache.rs`: `size_mismatch_leaves_no_cache_file_or_temporary_download` | behavior |
| 21 | A native command that never reads its declared stdin never polls the stdin source (no wasted work/blocking) | `runner/tests/command_client.rs`: `command_that_does_not_read_stdin_never_polls_the_source` | behavior |
| 22 | Terminal input still pending does not block a command's output or its completion signal | `runner/tests/command_client.rs`: `pending_terminal_input_does_not_block_output_or_completion` | behavior |
| 23 | Cancellation interrupts output that is currently blocked | `runner/tests/command_client.rs`: `cancellation_interrupts_blocked_output` | behavior |
| 24 | Losing the connection interrupts a caller that is blocked reading stdout | `runner/tests/command_client.rs`: `connection_loss_interrupts_a_caller_blocked_on_stdout` | behavior |
| 25 | `--help` on a native command never reads its declared stdin or calls back to the backend | `runner/tests/dispatch.rs`: `help_never_reads_declared_stdin_or_calls_backend` | behavior |
| 26 | A callback exit clears the "running" hint, and a revoked context can no longer dispatch | `runner/tests/dispatch.rs`: `callback_exit_clears_hint_and_revoked_context_cannot_dispatch` | behavior |
| 27 | Cancelling a dispatched command sends a callback cancel and clears its running hint | `runner/tests/dispatch.rs`: `cancellation_sends_callback_cancel_and_clears_running_hint` | behavior |
| 28 | A command declared as a shell builtin dispatches without needing a local command-service endpoint | `runner/tests/dispatch.rs`: `declared_shell_builtin_dispatches_without_a_local_endpoint` | behavior |
| 29 | A conversation's resident-service state retains services that no job currently owns, and releasing the conversation joins every resident's shutdown | `runner/tests/conversations.rs`: `conversation_state_retains_ownerless_services_and_release_joins_all_residents` | behavior |
| 30 | Concurrent acquisition of a resident service from multiple callers shares one service instance and every descriptor it handed out is checked (none dangling) | `demi-commands/tests/pool.rs`: `concurrent_acquisition_shares_service_and_checks_every_descriptor` | behavior (this test lives in the `demi-commands` crate but exercises `demi_runner::commands::cache`/`native::Services`, i.e. S3's pool; also needs N1's command-service client and N2's file commands as the concrete workload) |

Fixtures/goldens: temp git repositories built per test; no checked-in golden
files found for S3.

---

## T0

New infrastructure (differential corpus harness); no old TS/Rust test files
map here directly — the old product tested utilities only through S1's
`utilities_cat.rs`/`utilities_catalog.rs` (dispatch-only, see S1 row 15–16)
and through GNU tools as an out-of-repo comparison
(`docs/bash-behavior-comparison`, referenced by Gate A3). T0's own
fixtures/goldens (to be built, not ported): GNU coreutils 9.4, grep 3.11,
sed 4.9, findutils 4.9, diffutils 3.10 outputs, plus ripgrep and jq
reference outputs, under `testdata/gnu/<tool>/`.

---

## T1a — Text (cat, head, tail, wc, tee, sort, uniq, cut, tr)
## T1b — Search (grep, rg, find, xargs)
## T1c — Transform (sed, jq)
## T2a — Files (ls, cp, mv, rm, mkdir, rmdir, touch, stat, du, df, chmod, chown, realpath, mktemp)
## T2b — Misc (basename, dirname, env, seq, date, sleep, paste, nl, tac, od)
## T2c — diff, cmp

No old test file is owned by any of T1a–T2c: the baseline vendors GNU/uutils
implementations (`vendor/uu_*`, `vendor/{sed,jaq}`, `vendor/diffutils`,
`vendor/ripgrep`) and never wrote per-utility Demi tests against them; the
only old coverage of utility *dispatch* (not per-utility correctness) is S1
rows 15–16 above. Each of these WPs is judged entirely by:

- the differential corpus T0 builds (byte-identical stdout, exit code,
  stderr-presence, core + extended option sets — Gate B's own criteria), and
- S1's exec-handler contract (row 16) once the utility is registered.

No behaviors to port; no fixtures inherited beyond what T0 supplies.

---

## N1

Owns: command-service SDK (HTTP/2 over stdio, record framing, invocation
exchange, bounded IO, cancellation, edit journal with file lock).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Interleaved concurrent jobs each keep their own edit-journal segments separate | `command-service/src/edits.rs`: `interleaved_jobs_keep_their_own_segments` | behavior |
| 2 | Separate recorder handles for the same job share one underlying journal file | `command-service/src/edits.rs`: `separate_recorder_handles_share_the_job_journal` | behavior |
| 3 | Files that were restored to their original content, or removed, are omitted from the edit journal (no-op edits are not recorded) | `command-service/src/edits.rs`: `restored_and_removed_files_are_omitted` | behavior |
| 4 | An operation that errors partway through can still leave a real, recorded edit for what it did write | `command-service/src/edits.rs`: `an_error_can_leave_a_real_edit` | behavior |
| 5 | Failing to open a file that is too large to journal is not itself recorded as an edit | `command-service/src/edits.rs`: `failed_open_of_large_file_is_not_an_edit` | behavior |
| 6 | Binary file edits are journaled with no captured contents (metadata only) | `command-service/src/edits.rs`: `binary_edits_have_no_contents` | behavior |
| 7 | Calls held at or beyond the concurrency limit still all eventually start and finish (no lost call) | `command-service/tests/concurrency.rs`: `held_calls_beyond_any_count_all_start_and_finish` | behavior |
| 8 | Cancelling one call never turns away the next caller waiting for a slot | `command-service/tests/concurrency.rs`: `cancelling_a_call_never_turns_away_the_next` | behavior |
| 9 | Output nobody has read yet on one call does not hold back an independent, unrelated call | `command-service/tests/concurrency.rs`: `unread_outputs_never_hold_back_an_independent_call` | behavior |
| 10 | Abandoning a burst of concurrent calls keeps the underlying connection usable afterward | `command-service/tests/concurrency.rs`: `abandoning_a_burst_of_calls_keeps_the_connection` | behavior |
| 11 | Many callers issued back-to-back all succeed | `command-service/tests/concurrency.rs`: `many_callers_back_to_back_all_succeed` | behavior |
| 12 | The conversation endpoint grants no capabilities by itself, and validates the caller's identity before releasing a conversation | `command-service/tests/conversation.rs`: `conversation_endpoint_has_no_grants_and_validates_release_identity` | behavior |
| 13 | Cancelling a conversation joins its cleanup hook, and a cleanup failure still retires the service | `command-service/tests/conversation.rs`: `conversation_cancellation_joins_hook_and_cleanup_failure_retires_service` | behavior |
| 14 | Checking conversation status validates the conversation's identity | `command-service/tests/conversation.rs`: `conversation_status_checks_conversation_identity` | behavior |
| 15 | Framing correctly decodes messages split at every possible fragmentation boundary and preserves binary payloads exactly | `command-service/tests/framing.rs`: `decodes_every_fragmentation_boundary_and_preserves_binary` | behavior |
| 16 | An oversized frame header is rejected before its payload even arrives | `command-service/tests/framing.rs`: `rejects_oversized_header_before_payload_arrives` | behavior |
| 17 | An invocation requires an explicit completion record and rejects any record sent after it (trailing records) | `command-service/tests/framing.rs`: `requires_completion_and_rejects_trailing_records` | behavior |
| 18 | A completion record with an invalid exit code is rejected at the wire boundary, not deeper in application logic | `command-service/tests/framing.rs`: `rejects_invalid_completion_exit_codes_at_the_wire_boundary` | behavior |
| 19 | HTTP/2 works correctly over a Unix domain socket transport: streams before EOF and shuts down cleanly | `command-service/tests/local_transports.rs`: `http2_over_unix_socket_streams_before_eof_and_shuts_down` | behavior |
| 20 | HTTP/2 works correctly over a pair of stdio pipes as the transport: same guarantee | `command-service/tests/local_transports.rs`: `http2_over_two_stdio_pipes_streams_before_eof_and_shuts_down` | behavior |
| 21 | Recording an edit for a file that is currently open elsewhere waits for that file to be released, rather than failing or corrupting | `command-service/tests/open_files.rs`: `recording_an_edit_waits_for_an_open_file` | behavior |
| 22 | A native package descriptor is validated and content-hashed identically to a checked-in TypeScript fixture of the same package | `command-service/tests/package.rs`: `validates_and_hashes_typescript_package_fixture` | behavior (needs the TS package fixture as a golden, see below) |
| 23 | Generated (de)serialization for package values enforces the declared constraints | `command-service/tests/package.rs`: `generated_deserialization_enforces_package_value_constraints` | internal (schema-generation correctness; covered end-to-end by row 22) |
| 24 | Generated invocation validation checks nested values and optional/nullable fields | `command-service/tests/package.rs`: `generated_invocation_checks_nested_values_and_optional_nulls` | internal (schema-generation correctness) |
| 25 | Concurrent binary echo and a concurrent cancel both preserve the underlying connection | `command-service/tests/service.rs`: `concurrent_binary_echo_and_cancel_preserve_connection` | behavior |
| 26 | Resetting a call interrupts output that was blocked on flow control | `command-service/tests/service.rs`: `reset_interrupts_flow_control_blocked_output` | behavior |
| 27 | The SDK client keeps other calls live while one call's output is blocked | `command-service/tests/service.rs`: `sdk_client_keeps_other_calls_live_while_one_output_is_blocked` | behavior |

Fixtures: a checked-in TypeScript package descriptor fixture that row 22's
hash must match (`command-service/tests/package.rs` references it — locate
and carry forward exactly, it is the cross-language golden for package
identity).

---

## N2

Owns: demi-commands file commands (read, create, edit, patch) and operation
routing.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | If a multi-file patch's later write fails, the files it already wrote are restored to their earlier content (atomic across files) | `demi-commands/src/patch.rs`: `later_write_failure_restores_earlier_files` | behavior |
| 2 | The resident command executable correctly runs every builtin file operation (read, create, edit, patch) end to end | `demi-commands/tests/commands.rs`: `resident_executable_runs_all_builtin_file_operations` | behavior |
| 3 | A verified native artifact launches and retires correctly through the runtime | `demi-commands/tests/commands.rs`: `verified_artifact_launches_and_retires_through_runtime` | behavior (general native-launch contract shared with N1/S3, not files-specific; kept here because it lives in the same test file as row 2) |

Note: `demi-commands/tests/commands.rs` also contains
`conversation_browser_commands_share_state_and_retire`, which is a browser
behavior — see N3 row.

Fixtures: none beyond in-test tempdirs.

---

## N3

Owns: demi-commands browser (chromedp driver, tab registry, observations,
operations, cleanup).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The capture extension survives a forced reload: pages are preserved and its background worker is recreated | `demi-commands/src/browser/cdp.rs`: `capture_extension_reload_preserves_pages_and_recreates_its_worker` | behavior |
| 2 | The capture extension runs alongside the agent's pages and never itself appears as a visible tab | `demi-commands/src/browser/environment.rs`: `the_capture_extension_runs_beside_the_pages_and_is_never_a_tab` | behavior |
| 3 | A process that fails to retire still has its profile and failure cause retained (not silently lost) | `demi-commands/src/browser/environment.rs`: `failed_process_retirement_retains_the_profile_and_cause` | behavior |
| 4 | Browser output streams keep independent read cursors per subscriber, and correctly report both count and byte-eviction under a cap | `demi-commands/src/browser/history.rs`: `browser_streams_keep_independent_cursors_and_report_count_and_byte_eviction` | behavior |
| 5 | Keyboard input mapping: printable keys carry their text (so the page gets a `keypress`); Enter sends a carriage return; keys with no character stay raw; a Mac-style viewer maps Cmd to Control-equivalents on a Linux Host, and vice versa for text navigation; option/AltGr characters still type correctly; other viewers keep their own modifiers; Cmd+click becomes Ctrl+click on a Linux Host; a macOS-hosted viewer receives editing-command shortcuts for menu actions | `demi-commands/src/browser/keyboard.rs`: `printable_keys_carry_their_text_so_the_page_gets_keypress`, `enter_types_a_carriage_return`, `keys_without_a_character_stay_raw`, `a_mac_viewer_uses_control_shortcuts_on_a_linux_host`, `mac_text_navigation_maps_to_the_linux_keys`, `option_and_altgr_characters_are_still_typed`, `other_viewers_keep_their_modifiers`, `command_click_becomes_control_click_on_a_linux_host`, `a_macos_host_receives_editing_commands_for_menu_shortcuts` | behavior |
| 6 | The capture extension's manifest key is its stable extension id; the browser's reported user agent carries the real major version and strips the headless marker | `demi-commands/src/browser/launch.rs`: `the_capture_extension_id_is_its_manifest_key`, `the_user_agent_carries_the_major_version_and_no_headless_token` | behavior |
| 7 | A URL pattern used to scope a browser operation is matched as a literal glob | `demi-commands/src/browser/navigation.rs`: `literal_browser_url_globs` | behavior |
| 8 | Accessibility-tree observation omits node types it does not support without dropping the surrounding nodes | `demi-commands/src/browser/observation.rs`: `ax_values_omit_unsupported_types_without_losing_nodes` | behavior |
| 9 | Cause propagation: a deadline cause survives being wrapped by cleanup code; a controller-side deadline is reported as a timeout; a transport-end error keeps its cause while an explicit close is reported as "closed"; connection loss after dispatch is reported as an unknown outcome (not falsely success or failure); browser errors keep both their cause and how much input they had processed | `demi-commands/src/browser/operation.rs`: `deadline_cause_survives_cleanup_wrappers`, `controller_deadline_cancellation_is_a_timeout`, `transport_end_retains_its_cause_while_explicit_close_stays_closed`, `connection_loss_after_dispatch_has_an_unknown_outcome`, `browser_errors_keep_causes_and_input_progress` | behavior |
| 10 | Truncated stream output does not silently skip the entries that were omitted (it says something was cut); text errors show progress plus per-item detail; inspection output renders nested state/value trees; navigation results require a URL but tolerate missing metadata | `demi-commands/src/browser/output.rs`: `stream_output_truncation_does_not_skip_the_omitted_entries`, `text_errors_show_progress_and_individual_details`, `inspect_text_renders_nested_states_and_values`, `navigation_results_require_url_but_allow_missing_metadata` | behavior |
| 11 | A detached helper process fixture behaves as a real detached process for retirement tests; retirement only includes helpers marked as belonging to another session | `demi-commands/src/browser/process.rs`: `detached_helper_fixture`, `retirement_includes_marked_helpers_in_another_session_only` | behavior |
| 12 | The observed viewport ratio never exceeds what the capture actually encodes, and a captured picture always has an even number of device pixels (video-encoder constraint) | `demi-commands/src/browser/viewport.rs`: `a_ratio_stays_within_what_the_capture_encodes`, `a_picture_has_even_device_pixels` | behavior |
| 13 | Ending a browser session correctly runs its contract and cleanup | `demi-commands/tests/browser.rs`: `browser_contract_and_cleanup` | behavior |
| 14 | Exporting page assets captures observed content and still reports partial success if some assets fail | `demi-commands/tests/browser_assets.rs`: `assets_export_observed_content_and_keep_partial_success` | behavior |
| 15 | The asset inventory covers assets across cross-process iframes and expires entries when the child frame navigates away | `demi-commands/tests/browser_assets.rs`: `asset_inventory_covers_cross_process_frames_and_expires_on_child_navigation` | behavior |
| 16 | CDP session lifecycle: an oversized observation ends that browser but allows a fresh one to open; CDP calls are validated by method and scope, correctly address children, and preserve event cursors across calls; a CDP wait's expiry retains subscriptions, and cancelling an invocation releases connections; CDP eviction marks truncation and expires worker handles; detaching a debugger releases only that caller's hold, and timeouts correctly identify other still-attached debug owners; closing a tab joins any paused debug connections on it while leaving other tabs untouched; waiting on an element and its input correctly resample nodes that were inserted mid-resolution | `demi-commands/tests/browser_cdp.rs`: `oversized_observation_ends_the_browser_and_allows_a_fresh_open`, `cdp_validates_methods_scopes_children_and_preserves_event_cursors`, `cdp_wait_expiry_retains_subscriptions_and_invocation_cancellation_releases_connections`, `cdp_eviction_marks_truncation_and_worker_handles_expire`, `cdp_detach_releases_only_its_caller_and_timeouts_identify_other_debug_owners`, `tab_close_joins_paused_debug_connections_and_preserves_other_tabs`, `element_wait_and_input_resample_nodes_inserted_during_locator_resolution` | behavior |
| 17 | The clipboard round-trips raw text, HTML, PNG image data, and access through the page's own clipboard API | `demi-commands/tests/browser_clipboard.rs`: `clipboard_roundtrips_raw_text_html_png_and_page_api` | behavior |
| 18 | Downloads publish complete files and support media types; a cancelled streaming download never publishes partial output | `demi-commands/tests/browser_download.rs`: `downloads_publish_complete_files_and_support_media`, `cancelled_streaming_download_never_publishes_output` | behavior |
| 19 | Fetching multiple resources preserves the caller's input order and releases the tabs used for the batch; closing or cancelling a fetch releases its registered tabs | `demi-commands/tests/browser_fetch.rs`: `fetch_returns_input_order_and_releases_batch_tabs`, `fetch_closure_and_cancellation_release_registered_tabs` | behavior |
| 20 | Pages see an ordinary Chrome browser reporting the real user's time zone and languages; the agent's viewport sets the device pixel ratio, and screenshots stay expressed in CSS pixels regardless | `demi-commands/tests/browser_fidelity.rs`: `pages_see_an_ordinary_chrome_in_the_users_time_zone_and_languages`, `the_agents_viewport_sets_the_pixel_ratio_and_screenshots_stay_in_css_pixels` | behavior |
| 21 | Native form fill and text replacement work on real page elements; targeted keyboard input preserves selection and stops if focus is lost; `check`/`select` verify state and wait for options to be ready in order; action preconditions correctly detect shadow-DOM hits and share wait state across actions; label text and accessible name are treated as distinct, and ambiguity between them is surfaced explicitly; explicit navigation tracks documents, failures and history boundaries; URL-based observation delivers input and observes transient matches; inspection keeps literal `false` values, protects password fields, and handles expiry; input/animation probes clean up correctly on cancellation and on dialogs; text locators scan large documents, shadow roots and frames; an unregistered popup survives its opener tab closing; the element catalog supports pattern queries, pagination and read-only elements, untargeted key/selection/drag actions and console cursors, cross-origin frame scoping for focus/references, and pointer actions that wait for composited scroll; catalog probing records native form state on the server; catalog load-wait tracks only the current document; dialogs reject actions that are absent or inapplicable; viewport overrides are per tab and survive screenshots | `demi-commands/tests/browser_repairs.rs` (19 tests, see file for exact names) | behavior (large integration suite for the element/locator/dialog/catalog contract — merged; see file for full 19 test names if a finer split is wanted later) |
| 22 | A canceled browser launch reaps its helper processes before removing the profile directory; the Chrome process tree and its profile retire together; releasing a conversation cancels only that conversation's commands and retires only its profile; browser identity uses the trusted conversation/caller even when the invoking script's environment claims otherwise; fixture assertions correctly retire Chrome and profiles even before a panic resumes; closing the last tab fails any still-running command on it as "browser lost"; opening a new tab after a Chrome crash recovers without replaying stale tabs | `demi-commands/tests/browser_retirement.rs` (7 tests, see file for exact names) | behavior |
| 23 | Uploading attaches the chosen files and cleans up the file-chooser observation afterward | `demi-commands/tests/browser_upload.rs`: `upload_attaches_files_and_cleans_chooser_observation` | behavior |
| 24 | The native WebMCP bridge validates calls and invalidates its cache when the page's declared MCP surface changes | `demi-commands/tests/browser_webmcp.rs`: `native_webmcp_validates_calls_and_invalidates_changed_declarations` | behavior |
| 25 | Browser commands within one conversation share state and are retired together | `demi-commands/tests/commands.rs`: `conversation_browser_commands_share_state_and_retire` | behavior |
| 26 | Releasing a conversation cancels a browser command that is currently blocked producing output | `demi-commands/tests/conversations.rs`: `release_cancels_a_browser_command_blocked_on_output` | behavior |

Fixtures: real Chrome for Testing is required for several of these
(`crates/demi-commands/src/browser/cdp.rs`'s test is `#[ignore]`d without
`DEMI_TEST_CHROME`); HTML fixture pages served in-test via `Bun.serve`/local
HTTP servers, no checked-in golden files found.

---

## N4

Owns: live view (capture extension, page observers, streaming).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | A CPU without SVE (only SME) cannot be used for hardware capture — falls back correctly | `demi-commands/src/browser/live/capture.rs`: `a_cpu_with_sme_but_no_sve_cannot_capture` | internal (a capability-detection guard; the real behavior is "capture degrades gracefully without the right CPU features", better exercised by an integration test if one exists) |
| 2 | A watched tab's stream arrives H.264-encoded and starts with a key frame | `demi-commands/src/browser/live/capture.rs`: `a_watched_tab_arrives_as_h264_starting_with_a_key_frame` | behavior |
| 3 | Video frames split across network chunks are correctly reassembled, and a frame that fits in one chunk is not needlessly split | `demi-commands/src/browser/live/frames.rs`: `frames_split_across_chunks_and_join_within_one` | behavior |
| 4 | A frame the page cannot actually send correctly ends the stream (rather than hanging or silently dropping) | `demi-commands/src/browser/live/frames.rs`: `a_frame_the_page_cannot_send_ends_the_stream` | behavior |
| 5 | Adaptive bitrate: short scrolling bursts raise quality without needing a warm-up period; recovery after congestion waits for feedback and probes conservatively; the first bandwidth budget scales with device pixel count without an artificial ceiling; an idle picture is not evidence of available bandwidth; a long round trip alone keeps quality, but a stalled acknowledgment does not; low encoder demand (a simple animation) does not raise the budget; intermittent frames do not hide real demand while scrolling; congestion lowers bits, then frame rate, then resolution in order, and recovery reverses that order | `demi-commands/src/browser/live/rate.rs` (8 tests, see file for exact names) | behavior (the adaptive-bitrate control loop's full contract, kept as one merged row since the 8 tests describe one state machine) |
| 6 | A viewer can watch a tab and type into it alongside the agent; UI modes follow whichever of viewer/agent is acting; dialogs, form controls, file choosers and the clipboard all reach the viewer; a live view waits for the browser to exist and ends when its last tab closes; the user can immediately close a tab that is still loading; the user's requests answer without waiting for a page to finish loading; a viewer that ends releases only what it itself holds; two viewers sharing a tab: the last one to act decides state; a narrow still picture still matches the page's real coordinates; a watched tab's stream reflects the detail level of that viewer's own pixel ratio | `demi-commands/tests/browser_live.rs` (10 tests, see file for exact names) | behavior (the live-viewer contract; merged, one state machine) |

Fixtures: real Chrome for Testing where hardware capture is exercised; no
checked-in golden video/image files found in these test files (frames are
generated/synthesized in-test).

---

## N5

Owns: demi-claude (the Claude CLI installer/updater binary).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | `ensure` installs the executable and writes its install receipt | `demi-claude/src/tests.rs`: `ensure_installs_the_executable_and_its_receipt` | behavior |
| 2 | A second `ensure` for the same version does not re-download | `demi-claude/src/tests.rs`: `a_second_ensure_does_not_download` | behavior |
| 3 | A changed installation (content differs from the receipt) is replaced | `demi-claude/src/tests.rs`: `a_changed_installation_is_replaced` | behavior |
| 4 | A download whose digest does not match the manifest installs nothing | `demi-claude/src/tests.rs`: `a_wrong_digest_installs_nothing` | behavior |
| 5 | A downloaded body longer than its declared size installs nothing | `demi-claude/src/tests.rs`: `a_body_longer_than_its_size_installs_nothing` | behavior |
| 6 | A downloaded body shorter than its declared size installs nothing | `demi-claude/src/tests.rs`: `a_body_shorter_than_its_size_installs_nothing` | behavior |
| 7 | A release record with no entry for the current platform is reported as unsupported | `demi-claude/src/tests.rs`: `a_record_without_this_platform_is_unsupported` | behavior |
| 8 | Installing a new version removes the old one | `demi-claude/src/tests.rs`: `a_new_version_removes_the_old_one` | behavior |
| 9 | A preinstalled version already present is used without downloading anything | `demi-claude/src/tests.rs`: `a_preinstalled_version_is_used_without_downloading` | behavior |
| 10 | A preinstalled version that does not match what was requested is ignored, but kept on disk (not deleted) | `demi-claude/src/tests.rs`: `a_mismatching_preinstalled_version_is_ignored_and_kept` | behavior |
| 11 | Concurrent `ensure` calls for the same version download only once | `demi-claude/src/tests.rs`: `concurrent_ensures_download_once` | behavior |
| 12 | Cancellation stops an in-flight download and installs nothing | `demi-claude/src/tests.rs`: `cancellation_stops_a_download_and_installs_nothing` | behavior |
| 13 | Malformed install-receipt records are treated as invalid (not silently trusted) | `demi-claude/src/tests.rs`: `malformed_records_are_invalid` | behavior |
| 14 | The platform key used to look up a release follows the actual running machine, not a hardcoded value | `demi-claude/src/tests.rs`: `the_platform_key_follows_the_machine` | behavior |
| 15 | Versions are ordered semantically (semver-correct, not lexical) | `demi-claude/src/tests.rs`: `versions_order_semantically` | behavior |
| 16 | Status lists installed versions newest first | `demi-claude/src/tests.rs`: `status_lists_installations_newest_first` | behavior |
| 17 | The command-service front for demi-claude answers exactly one document per invocation | `demi-claude/src/tests.rs`: `the_service_answers_one_document_for_each_invocation` | behavior |

Fixtures: none beyond in-test HTTP mocks and tempdirs.

---

## M1

Owns: machine manager (machines wire server, device workers, gVisor
lifecycle, images, volumes, networking). M0 is feasibility work, no old
tests.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Obsolete gVisor settings and malformed or insufficient network pools are rejected at configuration time | `machines/src/__tests__/gvisor.test.ts`: "reject obsolete settings and malformed or insufficient network pools" | behavior |
| 2 | Network slots handed to machines are disjoint, and a released slot is reusable | `machines/src/__tests__/gvisor.test.ts`: "slots have disjoint networks and are reusable after release" | behavior |
| 3 | Managed credentials reject fields an untrusted caller injected, and malformed values | `machines/src/__tests__/gvisor.test.ts`: "managed credentials reject injected fields and malformed values" | behavior |
| 4 | Making a home ext4 image and later growing it round-trips correctly (`makeHomeImage`, `recoverVolume`), verified against `debugfs`/`e2fsck` | `machines/src/__tests__/home-image.test.ts`: "make and grow a home image round trip" (dynamic `test`-alias name, gated on `missingImageTools()` — needs `debugfs`/`e2fsck`/`unshare` installed) | behavior |
| 5 | A restrictive manager umask still permits creating a new Cloud root, and does not change later user-set modes | `machines/src/__tests__/home-image.test.ts`: "a restrictive manager umask permits a new Cloud root and preserves later user modes" (privileged, root-only, gated on `DEMI_GVISOR_STORAGE_E2E=1`) | behavior (requires root + Linux namespaces; not runnable in ordinary CI) |
| 6 | Publishing a disk image partially still preserves the last complete generation pair, and keeps only two committed generations | `machines/src/__tests__/image-store.test.ts`: "partial disk publication preserves the complete pair and retains only two committed generations" | behavior |
| 7 | A JSON-serialization failure during publication preserves the previously published file and removes any staged partial write | `machines/src/__tests__/image-store.test.ts`: "failed JSON serialization preserves the published file and removes staging" | behavior |
| 8 | Every provisioner call crosses the manager's socket with its arguments and its result intact | `machines/src/__tests__/wire.test.ts`: "every provisioner call crosses the socket with its arguments and result" | behavior |
| 9 | A failing call rejects with the manager's own error message, and the connection stays usable afterward | `machines/src/__tests__/wire.test.ts`: "a failing call rejects with the manager's message; the connection stays usable" | behavior |
| 10 | Concurrent calls are correctly matched to their responses by id, regardless of completion order | `machines/src/__tests__/wire.test.ts`: "concurrent calls are answered by id, whatever order they finish in" | behavior |
| 11 | A machine death event reaches every listener on every open connection | `machines/src/__tests__/wire.test.ts`: "a death reaches every listener on every connection" | behavior |
| 12 | Closing the wire connection reconciles state with the manager and disconnects; the next call reconnects automatically | `machines/src/__tests__/wire.test.ts`: "close reconciles the manager and disconnects; the next call reconnects" | behavior |
| 13 | If the manager process goes away, in-flight calls fail, and later calls dial it again | `machines/src/__tests__/wire.test.ts`: "a manager that goes away fails the calls in flight and is dialed again later" | behavior |
| 14 | A malformed wire frame drops only that connection, without disturbing the provisioner itself | `machines/src/__tests__/wire.test.ts`: "a malformed frame drops that connection without touching the provisioner" | behavior |

Fixtures: none beyond in-test sockets/tempdirs; row 4–5 need real
`debugfs`/`e2fsck`/`unshare` tools and (row 5) root — record as an
environment dependency for Go port acceptance, matching Gate 0's own note
that some baseline behaviors need real gVisor/root.

---

## P1

Owns: provider core (types, HTTP/SSE streaming, credential pools/refresh,
quota, models.dev catalog, model selection, response wires, scripted
provider).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Chat-Completions-style streaming maps split text, tool-call argument fragments and usage into unified provider events | `provider/src/__tests__/chat-completions-stream.test.ts`: "maps split text, tool call arguments, and usage" | behavior |
| 2 | The stream maps the vendor-compatible `reasoning_content` field once thinking output starts | same file: "maps the compatible reasoning_content field once thinking starts" | behavior |
| 3 | Malformed tool-call arguments are kept as the literal string the vendor sent, not repaired or dropped | same file: "keeps malformed tool arguments as the string the vendor sent" | behavior |
| 4 | A vendor error ends the stream, naming the vendor; a vendor error with no message falls back to the vendor's own name | same file: "ends the stream on a vendor error, naming the vendor", "falls back to the vendor name when the error carries no message" | behavior |
| 5 | A stream that ends without the `[DONE]` sentinel is still closed correctly | same file: "closes a stream that ended without the [DONE] sentinel" | behavior |
| 6 | An aborted signal ends the stream with an explicit abort event | same file: "ends the stream with an abort event once the signal aborts" | behavior |
| 7 | A malformed stream chunk is reported as a protocol error, not silently ignored | same file: "reports a malformed chunk as a protocol error" | behavior |
| 8 | Decoding a Chat Completions chunk accepts every null spelling vendors actually send, keeps tool-call increments in field order, and rejects a delta whose text/tool_calls field is the wrong type | `chat-completions.test.ts` (`decodeChatCompletionChunk`, 4 tests) | behavior |
| 9 | Token usage from a Chat Completions payload splits the cached-token prefix out of the prompt count, and reports zeros when the chunk carried no usage | `chat-completions.test.ts` (`tokenUsageFromChatCompletionsUsage`, 2 tests) | behavior |
| 10 | The file credential pool: reading metadata for a missing entry returns null; reads back exactly what was written; rejects metadata missing a label, with a non-string `updatedAt`, or that is not JSON; ignores a stray file sitting beside real entries; lists nothing before anything is written | `credentials-pool.test.ts` (`FileCredentialPool.readMeta`, first 7 tests) | behavior |
| 11 | A stored secret is only overwritten over the exact revision it was read at (optimistic concurrency); a composed pool falls back to a secondary while the primary has no account, and prefers the primary once it does; work queued under one key takes turns and one failure does not block the next | `credentials-pool.test.ts` (last 3 tests) | behavior |
| 12 | HTTP error classification: an error code is derived from the HTTP status; a message is categorized by keyword with a fallback to its code; transport-level transient failures classify as "overloaded" | `http.test.ts` (`httpErrorCode`, `normalizeErrorCode`, 3 tests) | behavior |
| 13 | An unknown provider failure with no response record still becomes a well-formed error event | `http.test.ts` (`providerErrorFromUnknown`) | behavior |
| 14 | Auth-status detection authenticates via a key, or via a matching auth header, and reports a labeled message when unauthenticated | `http.test.ts` (`authStatusFromKey`, 3 tests) | behavior |
| 15 | A failed HTTP request keeps the vendor's own text as the message and the whole response as the failure record; a non-JSON body is kept as-is, with no retry-wait unless a header names one | `http.test.ts` (`httpRequestFailedEvent`, 2 tests) | behavior |
| 16 | Reading a retry delay: both spellings of `Retry-After` are read, counted from when the failure was recorded; no wait is named without the header, for a stream record, or when the record can't be read; the wait is only set on an error event whose reader actually names a time | `http.test.ts` ("reading a failure record", 3 tests) | behavior |
| 17 | A catalog model maps into a model selection correctly: attachment extensions are omitted when unsupported, video-only models are accepted, an explicit extension override is respected, thinking config and service tier pass through, and a missing catalog entry falls back to id/options | `model-selection.test.ts` (`modelSelectionFromCatalog`, 6 tests) | behavior |
| 18 | Thinking capability reporting: nothing without a model, "disabled" when reasoning is unsupported, and the supported effort levels otherwise | `model-selection.test.ts` (`thinkingCapabilitiesFromProviderModel`, 3 tests) | behavior |
| 19 | An explicit models.dev catalog refresh revalidates a fresh catalog and preserves its content date on a 304; a forced refresh that fails still returns the old catalog, explicitly marked stale; a refresh with no old catalog rejects malformed external data and network failures | `models-dev.test.ts` (3 tests) | behavior |
| 20 | `defineProvider` exposes only the declared public provider fields and hides the runtime factory from serialization; it passes the execution-requirement capability flag through | `provider.test.ts` (2 tests) | behavior |
| 21 | `applyModelPolicy` remaps provider ids and applies include/exclude/default selection rules | `provider.test.ts`: "applyModelPolicy remaps provider ids and applies include, exclude, and default selection" | behavior |
| 22 | Provider quota: `createProviderQuota` probes and caches the latest result; probing throws when unsupported; percent-formatting helpers are correct; a probe and a live observation reconcile with each other; a persisted snapshot file carries the latest snapshot to the next provider instance (without the raw payload) until the account changes | `quota.test.ts` (5 tests) | behavior |
| 23 | The Responses-API event mapper streams thinking/text/tool-calls/usage; emits final reasoning text only if no delta already streamed it; does not repeat text already streamed when the message item finishes; keeps the frame text as the failure record and the wait time from the provider's own reader; maps `failed`/`incomplete`/`error` events, naming the vendor; keeps the request/response ids a failure carries; ends with an abort event on signal | `responses-stream.test.ts` (`mapResponsesEvents`, 7 tests) | behavior |
| 24 | `mapResponsesStream` decodes frames, skips `[DONE]`, and reports the usage it read; closes a stream that ended without `response.completed`; reports a malformed mapped-event payload as a protocol error | `responses-stream.test.ts` (`mapResponsesStream`, 3 tests) | behavior |
| 25 | Decoding a Responses event ignores event types Demi does not map, keeps unread vendor fields, narrows a delta/function-call event by its type, rejects a malformed mapped payload or a message item with non-array content (naming the field), decodes an unknown output-item type to null instead of failing, and reads a malformed error payload as an error rather than a protocol failure | `responses.test.ts` (`decodeResponsesEvent`, 8 tests) | behavior |
| 26 | The Responses item schema rejects a reasoning-summary part with no text, and drops unknown message-content parts while keeping known ones | `responses.test.ts` (`responsesItemSchema`, 2 tests) | behavior |
| 27 | Token usage from a Responses payload splits the cached prefix out of the input count, reports zeros with no usage, and rejects a non-numeric usage count | `responses.test.ts` (`tokenUsageFromResponsesUsage`, 3 tests) | behavior |
| 28 | Server-Sent-Events framing: frames CRLF streams and keeps the event name; joins multi-line data fields per spec; passes `[DONE]` through as a payload; yields a final frame with no terminating blank line; reassembles frames split across chunks (including multi-byte text); drops a frame with no data field; stops at an aborted signal without yielding the partial frame; yields nothing for a body-less response | `sse.test.ts` (`readServerSentEvents`, 8 tests) | behavior |
| 29 | `StubProvider` yields its scripted events across turns, and throws once its scripted turns run out | `stub.test.ts` (2 tests) | behavior |

Fixtures: none beyond in-test mock HTTP servers/fetch stubs. No golden
files found.

---

## P2

Owns: Anthropic API, OpenAI API, Google providers.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The Anthropic API provider resolves its endpoint and API key from environment variables, and an explicit `baseUrl`/`apiKey` takes precedence over them | `provider-anthropic-api/src/__tests__/provider.test.ts`: "resolves endpoint and API key from env vars", "explicit baseUrl and apiKey take precedence over env vars" | behavior |
| 2 | A reused Anthropic runtime applies the correct output-token limit per request after a model switch | same file: "a reused Anthropic runtime uses the output limit of each request after a model switch" | behavior |
| 3 | The Anthropic model catalog mirrors Claude Code's defaults, and explicit configured models replace it | same file: "Anthropic API model catalog mirrors Claude Code defaults and explicit models replace it" | behavior |
| 4 | The Anthropic request body correctly groups adjacent user/tool_result and assistant/tool_use turns | same file: "request body groups user/tool_result and assistant/tool_use turns" | behavior |
| 5 | The Anthropic stream mapper maps thinking, text, tool use and usage events | same file: "stream maps thinking, text, tool use, and usage" | behavior |
| 6 | A `content_block_delta` without an index is a protocol error; event types this adapter does not map are ignored, but malformed known ones are not silently ignored | same file: 2 tests | behavior |
| 7 | Extended-thinking effort and adaptive configs map onto Anthropic's budget-token parameter instead of being dropped; thinking budgets clamp below `max_tokens`, and default `max_tokens` scales with the agent | same file: 2 tests | behavior |
| 8 | Gemini tool schemas are reduced to only the JSON-Schema keywords Gemini's API accepts | `provider-google/src/__tests__/provider.test.ts`: "tool schemas are reduced to the keywords Gemini accepts" | behavior |
| 9 | A malformed OpenAI stream event surfaces as a provider error | `provider-openai-api/src/__tests__/provider.test.ts`: "a malformed stream event surfaces as a provider error" | behavior |

Fixtures: none beyond in-test mock HTTP responses.

---

## P3a

Owns: Codex provider.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The file-backed Codex auth store resolves ChatGPT auth from the official `auth.json` shape, refreshes near-expiry tokens while preserving unknown fields, resolves a plain API key and reports missing auth without leaking secrets, and separates a malformed `auth.json` from a simply-missing login | `provider-codex/src/__tests__/auth.test.ts` (4 tests) | behavior |
| 2 | JWT helpers parse ChatGPT account claims conservatively; auth-mode resolution and redaction follow the official `auth.json` precedence rules | same file: 2 tests | behavior |
| 3 | The Codex credential pool: a store built for one account resolves it whichever account is currently active; a refresh that loses a concurrent write still uses the winner's tokens; a refresh the vendor refuses falls back to stored tokens if someone else refreshed, and fails if nobody did; refreshes of the same account serialize (later spends the earlier); an empty pool is unauthenticated unless composed with the vendor login; a provider standing for one account keeps its own usage when another account is selected/removed; an unreadable vendor login is reported as an error, not thrown | `credential-pool.test.ts` (8 tests) | behavior |
| 4 | Codex credentials: `importDefault`/`setActive`/pool-aware `resolve`; `createCodexProvider` exposes the credentials surface by default, and omits it with a custom auth store; `beginLogin` runs the device-code flow, surfaces pending material, and imports into the pool | `credentials.test.ts` (4 tests) | behavior |
| 5 | The Codex device-login flow surfaces pending material and assembles a vendor-shaped auth record; rejects a device-code response with no user code; reports unsupported servers distinctly | `device-login.test.ts` (3 tests) | behavior |
| 6 | `listModels` forwards a refresh request and retains the configured model filter; the Codex catalog maps slug ids and capabilities, keeps full efforts/defaults in priority order, and rejects malformed metadata instead of silently dropping entries; the catalog client uses auth headers/client version, refreshes auth once after a 401, rejects `OPENAI_API_KEY` auth for the Codex-backend catalog, returns stale cache on non-auth failures, and an explicit refresh bypasses the TTL and reports failures as stale | `models.test.ts` (10 tests) | behavior |
| 7 | The public Codex provider config exposes only serializable fields; header/URL construction follows Codex's auth routing; the provider streams text/thinking/tool-calls/usage; it refreshes auth once on a 401 and retries; classifies transport timeouts as retryable "overloaded"; preserves HTTP failure diagnostics without retrying inside the provider itself; finds a usage-limit-lifts time in a stream frame, an envelope, or an HTTP body | `provider.test.ts` (first 7 tests) | behavior |
| 8 | `AgentSession` retries Codex server errors without replaying the user's turn; the auto-transport falls back to SSE only before the WebSocket transport has emitted anything; the WebSocket transport sends the correct beta header and finishes on `response.completed` even with no close frame; the provider integrates correctly with `AgentSession` and shell tools for function calls; provider-stream steers replay in a same-turn follow-up before queued sends drain | `provider.test.ts` (remaining 6 tests) | behavior (spans A2 `AgentSession`; needs A1/A2 to exercise) |
| 9 | Rate-limit headers map to primary/secondary windows; the quota probe reads the account correctly; a refused usage status or a keyless API-key account fail the probe cleanly; a quota-observation failure never interrupts actual inference | `quota.test.ts` (4 tests) | behavior |
| 10 | Building a Codex Responses request body converts inference items/tools/thinking/cache key; skips unsigned thinking and encodes tool-result images; writes a service tier only when one was selected; preserves catalog reasoning levels uncapped; the SSE stream decodes Responses events (and skips the rest) and reports a malformed mapped event; a Codex tool-use id carries both the call id and the item id | `responses.test.ts` (7 tests) | behavior |
| 11 | Real Codex CLI/network end-to-end scenarios | `real-codex.e2e.test.ts` (env-gated `DEMI_CODEX_E2E`/`DEMI_CODEX_CACHE_E2E`, no titles extracted — dynamic `e2e(...)`/`cacheE2e(...)` calls) | excluded — calls the real vendor over the network; per project rule an automated test never calls a real model, so this is not ported as a fixture test. Cover the same ground once, manually, during product acceptance with a live credential (Gate D's own acceptance clause). |

Fixtures: none beyond in-test HTTP mocks (except the e2e file above, which
needs a live Codex/ChatGPT account).

---

## P3b

Owns: Grok Build provider.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The file-backed Grok auth store resolves an OIDC session from the Grok CLI's `auth.json` shape, refreshes near-expiry tokens while preserving sibling entries, and reports missing auth without leaking secrets | `provider-grok-build/src/__tests__/auth.test.ts` (3 tests) | behavior |
| 2 | An entry with no access token is skipped, a mistyped field is an error; a refresh response with no access token is an error; `selectAuthEntry` prefers OIDC entries scoped to `auth.x.ai` | same file (3 tests) | behavior |
| 3 | The store steals an abandoned Grok CLI `auth.json.lock` and refreshes; it adopts a token another process already refreshed instead of refreshing again; a live lock is never abandoned on age alone | same file (3 tests) | behavior |
| 4 | The Grok credential pool: a pinned `credentialId` resolves that account, not the active one; a lost concurrent replace still uses the winner's stored tokens; a refresh kept by the document is stored; a refresh refused because the document changed uses the stored tokens; a refresh that waited behind another finds the tokens it should; an empty pool is unauthenticated unless composed with the vendor login; the vendor pool holds one account per login and follows the vendor; pinned credentials keep their quota when the active account changes | `credential-pool.test.ts` (8 tests) | behavior |
| 5 | Grok credentials import entries and switch the active one; `createGrokBuildProvider` exposes credentials by default | `credentials.test.ts` (2 tests) | behavior |
| 6 | The Grok device-login flow drives the official device-flow contract and assembles a vendor-shaped entry; seeds the team principal from the access token; fails fast on terminal OAuth errors | `device-login.test.ts` (3 tests) | behavior |
| 7 | Grok catalog reads contact the source including on explicit refresh; the provider posts chat completions with CLI session headers; refreshes once on HTTP 401 then retries; the chat body maps tools/tool-replay/reasoning effort and degrades video blocks to text (not `image_url`); the stream uses the shared Chat Completions mapper; a malformed model catalog is a protocol error; provider config rejects a misspelled key; the model catalog maps the Grok `/v1/models` payload | `provider.test.ts` (9 tests) | behavior |
| 8 | Grok quota probe: maps billing+subscription tier; prefers the credits-config percent and current period; observed rate-limit headers map short windows; the probe hits both credits-billing and subscription-user endpoints; a plan whose billing meters no credits has no credits window | `quota.test.ts` (5 tests) | behavior |

Fixtures: none beyond in-test HTTP mocks.

---

## P4

Owns: Claude Code provider (CLI through the Host).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | A valid OAuth document resolves to file-sourced access; a corrupt one is an error, never silently repaired; a missing one reads as unauthenticated; an expiring secret is renewed and the renewal written back; an invalid renewal is rejected before it reaches the document; a non-expiring secret is used as-is; a renewal that loses a concurrent write uses the stored winner; a refused renewal uses the tokens stored meanwhile, or is an error if the document was unchanged; the reported access names the source the store was given | `provider-claude-code/src/__tests__/auth.test.ts` (10 tests) | behavior |
| 2 | `buildClaudeArgs`/env match the planned CLI contract; a public env overlay applies last and wins over the OAuth token; building args for a request maps summary thinking effort to CLI flags | `cli.test.ts` (3 tests) | behavior |
| 3 | Claude credentials add/set-active and pool-aware resolve; keychain-sourced tokens are never injected as a CLI token; the env overlay carries `CLAUDE_CODE_OAUTH_TOKEN`; provider creation wires credentials and auth status (and a custom auth store skips the credentials surface); a pinned credential resolves whichever account is active; an empty pool stands for the environment token via the vendor pool behind it; a malformed keychain item is an error, never treated as a missing login; a machine with no vendor login has an empty vendor pool; the vendor pool itself refuses writes; a read-only document is never renewed even past expiry; `importDefault` copies the vendor login in as a stored secret; the default provider stands for the vendor login until an account is stored; pinned credentials keep quota when the active account changes; the provider can stand for the pinned account of an injected pool | `credentials.test.ts` (18 tests) | behavior |
| 4 | Converting inference items to Claude's stream-json input messages: groups assistant/tool_result turns, skips unsigned assistant thinking on replay, converts binary media to base64 sources, preserves unicode/long fields/mixed tool results | `jsonl-output.test.ts` (`requestToInputMessages`, 5 tests) | behavior |
| 5 | Mapping a Claude stdout message: maps assistant content/stream deltas/tool calls/usage; reports the last iteration's usage as the response total, not the turn total; handles empty content and thinking-boundary events; preserves provider error codes and result error text; rejects a malformed assistant `tool_use` block | `jsonl-output.test.ts` (`mapClaudeStdoutMessage`, 5 tests) | behavior |
| 6 | Decoding a Claude stdout message rejects a malformed payload of a type Demi reads, ignores (not errors on) a message type it does not read, and still reports an error message whose own `message` field is malformed | `jsonl-output.test.ts` (`decodeClaudeStdoutMessage` + 2, 3 tests) | behavior |
| 7 | `runClaudeCodeLogin` builds a PKCE authorize URL and exchanges the pasted code, and rejects a state mismatch; `refreshClaudeCodeSecret` renews tokens and keeps the refresh token across rotation gaps; a token response with no access token is a credential error; `expires_in` is read as a number however the endpoint spells it | `login.test.ts` (5 tests) | behavior |
| 8 | `listModels` forwards refresh and retains the configured model filter; `parseClaudeModelVersion` handles full ids and date snapshots; the models.dev catalog keeps full ids at/above a minimum version without family allowlists, and preserves explicit metadata while leaving missing capabilities null; listing models uses stale cache on network failure and never falls back to hardcoded models; catalog filtering keeps one shared snapshot per minimum version | `models.test.ts` (6 tests) | behavior |
| 9 | The public provider config accepts only serializable fields, and reports auth via the credential store while deferring runtime state; the provider streams text/response events; preserves cache-usage fields from result messages; handles an empty successful stream without leaking transport state; forwards context-overflow result errors with usage; handles `control_request` tool calls (including SDK-MCP) across run calls; exposes one MCP tool batch before answering any call; forwards `tool_result` images to the SDK MCP response as passthrough base64; renders prior tool calls as text on a cold start, never as structured `tool_use`; keeps one process alive across turns, sending only the new user message; a clone owns independent live-process state; rejects malformed SDK-MCP `tools/call` without entering pending state; handles assistant `tool_use` across run calls, fails fast when one has no matching `tool_result`, and reports malformed `tool_use` blocks as provider errors; terminates the active transport when consumers stop after a provider error; fails fast on a pending control_request with no matching tool_result; rejects malformed `tools/call` control requests without entering pending state; reports nonzero CLI exits that produce no result message; clears/terminates transport when stdout iteration fails; abort kills the active transport; a long turn keeps none of the handled stream lines and an abort still ends it; integrates with `AgentSession` and shell tools for `control_request` tool calls; keeps repeated MCP request ids distinct in `AgentSession`; cold-restarts on a mid-session model change; starts a fresh process for the next turn after switching accounts; an edit starts a fresh transport with no stale continuation | `provider.test.ts` (29 tests, see file for full list) | behavior (the whole run-lifecycle/process-management contract; some tests exercise `AgentSession` directly and so also validate A2) |
| 10 | Mapping Claude usage: `five_hour`/`seven_day` windows; `createClaudeCodeQuota` probes with a mock fetch; observed rate-limit headers and stream-message rate limits; a malformed limit entry is dropped while the rest of the snapshot renders; a non-object usage payload yields no windows | `quota.test.ts` (6 tests) | behavior |
| 11 | An injected spawn receives the right command/args/untranslated cwd/clean env; the transport round-trips stream-json over an injected spawn handle; the local default spawn runs and reaps a real child process | `transport.test.ts` (3 tests) | behavior |
| 12 | The wire log is on by default and relocatable; both spellings of "off" turn it off; any other value is a configuration error | `wire-log.test.ts` (3 tests) | behavior |
| 13 | Real Claude CLI end-to-end streaming/thinking scenarios | `real-cli.e2e.test.ts` (env-gated `DEMI_CLAUDE_CODE_E2E`/`..._CACHE_E2E`/`..._THINKING_E2E`, no titles extracted — dynamic `e2e(...)` calls) | excluded — calls the real `claude` CLI/model. Not ported as a fixture test; cover manually during product acceptance with a live credential. |
| 14 | The wanted Claude CLI version is resolved from the vendor's release feed; an invalid "latest" pointer or missing release fails instead of silently picking another version; a machine with no CLI waits for install, one with an older CLI answers with it while updating beside it; a failed "latest" install names why, with the version | `backend/src/__tests__/claude-cli.test.ts` (4 tests) — backend-side CLI install/version orchestration onto a Host | behavior (needs E4/managed-host wake and H1 Host access to actually install onto a device/Cloud) |
| 15 | A Claude Code account's entry reads its CLI version from the user's own configuration | `backend/src/__tests__/scenarios/claude-cli.test.ts`: "a Claude Code entry reads its CLI from the user's..." | behavior (same install/version concern as row 14, end-to-end through the backend) |

Fixtures: none beyond in-test HTTP/process mocks (except the two e2e files,
which need a live CLI/account).

---

## A1

Owns: agent core — types, protocol frames, transcript, store, node, tools.
(See note below on judgment calls splitting A1/A2/A3.)

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Message admission: a busy tool receives an ordered, durable batch in its own continuation with no separate human queue; an idle batch opens one internal continuation and a duplicate admission does not wake it again; a user abort retains unread input through restore until an explicit continuation; live provider steering and a committed replay retain the same source envelope; admission crossing a finishing turn is retained and wakes the next continuation; admission rejects malformed envelopes and mismatched recipients; retrying a failed internal continuation preserves the completed human turn and all its receipts; restoring durable idle input wakes exactly once, restoring already-materialized input never replays delivery; input admitted during a standalone compaction stays outside the summary and continues afterward | `agent/src/__tests__/agent-messages.test.ts` (9 tests) | behavior |
| 2 | `submit` confirms its own transcript entry before the model finishes; preserves a caller-supplied message id through queue admission; rejects when a stated outcome happens before confirmation; fails immediately on an already-detached client; `edit` waits for its own receipt even after a matching replacement appears, and distinguishes a safe correction from an uncertain outcome | `client-submit.test.ts` (6 tests) | behavior |
| 3 | Command-state store: atomic concurrent mutations preserve all values, return only the detached changed data, and skip unchanged maps; assistant cutoffs select the committed state, and editing restores the target user boundary; a rejected edit preserves current state and existing invocation handles; assistant completion is ordered after an already-admitted state commit; a failed or cancelled commit leaves the old head and releases subsequent mutations; disposal invalidates pending/future operations from a job; storage validates values and references without repairing missing history | `command-state.test.ts` (7 tests) | behavior |
| 4 | Media: a media block survives round-tripping through the blob store; the block schema accepts both wire forms of a media source; a corrupt stored reference is reported (not silently carried on); a missing blob degrades to a placeholder instead of failing | `media.test.ts` (4 tests) | behavior |
| 5 | Journal patches replicate appended blocks and streaming text deltas, and tool completion as a block replace; journal `add` values are snapshots unaffected by later mutation of the live block; a retry rewind emits a full replace and drops finer-grained history; `takePatches` returns null when nothing changed, and revisions only increase | `patch.test.ts` (5 tests) | behavior |
| 6 | Pending steers: snapshots hide internal wakeups without removing them from the execution queue; the client removes pending steers by id even when history arrives before the list; the client validates pending data and owns its own copies of binary attachments; a malformed server frame disconnects the client rather than being acted on | `pending-steers.test.ts` (4 tests) | behavior |
| 7 | The agent's root entry point does not export node-only stdio transports (browser/Node boundary) | `root-entry.test.ts` | behavior |
| 8 | Standard shell tool schemas do not expose model-controlled output budgets or offsets; the declared input schema and the actual invocation agree on what's accepted; the shell preview budget follows an 800k-context threshold; the shell tool result exposes output paths and a bounded preview without stdout body sections; preview truncation never splits a surrogate pair; a completed short `shell_exec` hides and releases its command handle, while a completed truncated one keeps it for artifacts; command handles are required only for running or over-budget output; the custom preview policy survives model changes and live-session takeover; a sniffed binary stream the model accepts is attached as a media block, one it does not accept explains why nothing was attached; unknown/truncated binary streams stay placeholder-only with a reason; media over its modality cap is withheld and points at a smaller version; a shell result with no binary stdout stays text-only | `tools.test.ts` (14 tests) | behavior |
| 9 | Streaming-delta persistence cost does not scale with transcript length; a turn with tool calls persists at each tool dispatch and the action boundary, not per event; the client resyncs with a full snapshot when the patch-revision stream has a gap; `append_text` patches replace their target block instead of mutating it | `transcript-pipeline.test.ts` (4 tests) | behavior |
| 10 | `TranscriptLog` appends user turns and provider text/response events, and steer blocks replayed in turn order; completes pending tool calls and emits exact tool inference items, including when tool ids repeat; bounds text for replay without splitting surrogate pairs, scrubbing lone surrogates even when text otherwise fits; replays thinking signatures and redacted thinking in provider order; safely stores non-JSON tool inputs; removes dangling executing tool calls; inserts a compaction boundary and replays from the latest one; a snapshot survives a JSON round-trip without changing replay or metadata, and is insulated from later live-block mutation; returns the latest extension-state snapshot, tolerating non-JSON state for token estimates; the context estimate anchors on the latest provider-reported usage, discards an anchor exceeding the context window as a contract violation, is invalidated by compaction after the last response, and includes images/documents in the unanchored estimate | `transcript.test.ts` (18 tests) | behavior |
| 11 | The tree store: a later turn saves only its own rows, never rewriting earlier ones; `create` queues the first message with the node and the journal replaces it once the turn is saved; a parent save acknowledges pending and materialized completions atomically; reopening makes an archived node live with its reviving message queued, and delete removes the whole subtree; a prior completion cannot mark a resumed round delivered | `tree-store.test.ts` (5 tests) | behavior |
| 12 | A stored transcript names the commands it last saw running; the shell-view schema refuses a view written by another tool | `summaries.test.ts` (2 tests) | behavior |
| 13 | The end-to-end demi shell-tool scenario: a stream past the view budget, a binary final stream, a non-zero exit with stderr, and a command outliving its observation window all report correctly through the tool's output-view contract | `backend/src/__tests__/scenarios/s2-output-view.test.ts` (4 tests) | behavior (needs A2/A4 and S1 to execute an actual shell command end to end) |
| 14 | Attaching a message with a binary/media body round-trips correctly from upload through the model to the blob route | `backend/src/__tests__/scenarios/s8-attachments.test.ts`: "upload → ref → bytes at the model → blob route" | behavior (needs E3's upload route and the backend's blob store; kept here as a tools/media-block behavior) |

Fixtures: none beyond in-test fixtures; no golden files.

Note on A1/A2/A3 split: the plan's reference paths
(`packages/agent/src/{types.ts,protocol,transcript,store,node,tools.ts}`
for A1, `.../session` for A2, `.../{subagent,server}` for A3) do not line
up one-to-one with the flat `src/__tests__/` test files. I assigned
`agent-messages`, `client-submit`, `command-state`, `media`, `patch`,
`pending-steers`, `root-entry`, `tools`, `transcript*`, `tree-store`,
`summaries` to A1 (protocol/transcript/store/tools concerns);
`compaction*`, `context-cache`, `editing`, `fork`, `host-routing`,
`host-switch-migration`, `recovery`, `session*`, `turn-retry` to A2
(session concerns, below); and `server`, `stdio-transport`,
`websocket-transport`, `subagent` to A3 (below). This is a judgment call,
flagged for the orchestrator to confirm or correct before A1/A2/A3 briefs
are written.

---

## A2

Owns: agent session.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | `resolveCompactionThreshold` uses a ratio when absolute tokens are unset, prefers absolute tokens when both are set, clamps absolute tokens to the context window, and disables compaction for a non-finite ratio | `compaction-threshold.test.ts` (4 tests) | behavior |
| 2 | Preflight compaction summarizes before the model request and keeps the incoming user message once; an absolute preflight threshold takes precedence over the ratio; retry/resume both trigger preflight compaction before rerunning/continuing; compaction never re-summarizes only the previous boundary summary; a summary provider error, or an aborted hanging summary, leaves no boundary/marker block; an empty summary is a no-op and the session stays usable; compaction summary input keeps completed tool_use/tool_result paired and keeps aborted text progress; a context-overflow error during summarization is atomic and classified when no smaller slice is available, and otherwise retries with a smaller slice (including via the streaming iterator); the summary preserves the structured request prefix; a clone runs the inherited tool loop without mutating parent state; multiple compactions replay only the latest boundary summary; manual compaction after an existing boundary summarizes only the latest replay window; a single oversized turn compacts at a block boundary without orphaning tool history; compaction is a no-op while a tool call is pending, or with nothing compressible | `compaction.test.ts` (21 of its 37 tests, the core compaction algorithm) | behavior |
| 3 | Aborting during preflight/retry-preflight/resume-preflight compaction stops before the model request and stays atomic; a send/retry/resume queued during preflight or full compaction drains/reruns/continues only after the original completes; auto compaction after a tool result resumes without re-executing the tool; auto compaction counts cache usage as context pressure and is bounded per turn (no storm at a too-low threshold); the compaction boundary/marker survive snapshot reconstruction; a steer during preflight compaction queues and lands on the same turn request; a steer during standalone compaction materializes and the next turn carries it | `compaction.test.ts` (remaining 16 tests) | behavior |
| 4 | The provider request keeps a stable prefix across ordinary turns; request text content is bounded without mutating the transcript's audit log; the stable prompt prefix is byte-identical for equivalent histories and only changes on cache-affecting inputs; the prefix restabilizes after compaction replaces old history; a replay compaction summary reuses the previous turn's prefix for provider caches; the request is built from the effective transcript, excluding internal blocks; retry/resume requests match `TranscriptLog.collectInferenceItems`; cache usage is recorded without leaking into model context or breaking the tool loop; context-overflow provider errors are explicit and keep the session recoverable; snapshot reconstruction preserves model-visible context exactly | `context-cache.test.ts` (10 tests) | behavior |
| 5 | Editing a message preserves only its prefix across memory, patches, storage and provider input; complete detached content preserves multiple texts/references/attachment bytes; a failure at any editing stage keeps accepted history and releases admission; a harness must explicitly provide state reconstruction, which preserves the shared root and discards suffix snapshots; a save barrier exposes no candidate history and behaves correctly on save failure; abort during preparation releases admission without adding an abort to history, abort during commit follows the durable outcome; dispose waits for an in-flight edit save before releasing the runtime; accepted edit ids stay idempotent across restart and a later removing edit; repeated in-flight operation ids share one acceptance and reject conflicting content; disposal failures during generation preserve the accepted replacement; edit admission excludes sends/steers/model-changes/wakeups during preparation; a pending wakeup blocks editing without being consumed; certain block kinds are never editable targets; a stale editor is rejected after another edit or a runtime restart; prefix mutations during preparation are rejected; references are re-resolved while editor content stays unexpanded; editing respects multiple compaction boundaries and a removed marker (which invalidates old measured usage); fixed-seed histories cut at the selected user and keep patches/reload identical; a provider-creation failure releases admission and preserves the consumed runtime; preflight compaction after an edit summarizes only retained history; an earlier in-flight checkpoint settles before the edit save and cannot restore removed rows; a failed continuation retains a completed tool/receipt and retrying cannot rerun the tool; a pending/first child-record admission excludes competing restoration until release; active generation and queued sends block editing without losing work | `editing.test.ts` (27 tests) | behavior |
| 6 | `Fork` captures its prefix before awaiting state reconstruction even if the source is edited meanwhile, and rejects a reconstruction that changes the retained prefix without changing the source | `fork.test.ts` (2 tests) | behavior |
| 7 | Action metadata can switch Host between turns while the same Host keeps its shell state; a command handle cannot be controlled from another action's Host | `host-routing.test.ts` (2 tests) | behavior |
| 8 | Switching Host between turns injects a context block and keeps one continuous transcript | `host-switch-migration.test.ts` | behavior |
| 9 | A turn whose only trace is a failure unwinds completely; emitted text, a tool call (whether or not completed), a response, or an abort each stop the unwind (they are real history); whitespace-only text is a leftover, not real output; only the latest turn is in play, an earlier one is settled; an empty transcript keeps everything and continues; resume reruns a turn that failed before producing anything, keeps a completed tool result and continues after it, keeps already-emitted text exactly once, and drops stale thinking so a rerun does not replay it | `recovery.test.ts` (11 tests) | behavior |
| 10 | Session lifecycle and control (this file's ~90 tests, merged into behavior groups): clone/checkpoint create independent sessions with correct provider/runtime/state handling; model switching correctly disposes the old provider and compacts with the old model only when the new window is smaller, mid-turn or not, same-provider or cross-provider; sends/retries/resumes write and record history exactly once, roundtrip tool results, and are queued/drained/dequeued/converted-to-steer correctly while a turn is active; steering (queued, active, provider-native, provider-stream) materializes at the correct point (before/after tool execution, at continuation) and is preserved or dropped correctly on abort/retry/resume; provider errors and thrown tool invocations are recorded and do not corrupt the transcript or stop queued sends; snapshot persistence (checkpoints, extension state) is serialized so writes never overlap and is insulated from later mutation; tool execution, progress events, and terminal-tool-result yield-wakeups correctly start/steer subsequent turns; abort correctly interrupts every blocking point (reference resolution, a non-yielding provider, a hanging compaction summary, a long tool call) without losing pending state; mutation guards reject conflicting reservations | `session.test.ts`, `session-marathon.test.ts`, `session-ownership.test.ts` (all tests in these three files, ~96 total) | behavior (this is the single largest behavior surface in the whole ledger; a finer breakdown by test name should be done by whichever agent implements A2, using this file list as the checklist) |
| 11 | Transient provider errors before any content retry silently, including through an empty thinking lifecycle; non-retryable error codes surface immediately; an error after streamed thinking retries and unwinds the thinking, but an error after streamed text does not retry (no duplicate output); retries stop at `maxAttempts` and surface the final error; a vendor wait beyond the backoff ceiling fails immediately as terminal; retry-policy helpers honor `Retry-After` and cap backoff; tool-call turns retry transient continuation failures; resume preserves completed tools after an exhausted provider error | `turn-retry.test.ts` (10 tests) | behavior |
| 12 | A cross-host command (subagent-adjacent) preserves the child node's storage scope; two sessions on one runner keep their cwd and state apart; a turn survives its client disconnecting | `backend/src/__tests__/scenarios/{s5-subagents (1 of its 3 tests), s7-concurrent-sessions, s9-detach}.test.ts` | behavior (backend-side session-isolation acceptance; needs H1's runner registry and E3's conversation wiring to run) |

Fixtures: none beyond in-test fixtures; no golden files.

---

## A3

Owns: subagents and agent server.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Tree mutation queues admitted actions and execution context persists per node; `AgentClient.open`/`send` run through `InProcessTransport` and emit transcript/phase frames; send metadata reaches server-side harness context; the client clears its local transcript view on close | `server.test.ts` (first 4 tests) | behavior |
| 2 | The server persists the tree root as a node with its journal in the tree store, resumes a conversation by session id restoring its transcript, and renders registered command help into the harness system-prompt context | `server.test.ts` (3 tests) | behavior |
| 3 | The server forwards provider error codes once and preserves the transcript error block; maps shell-tool progress into `shell_output` frames; bridges `shell_write` frames to the active shell command; stops a running command on `shell_abort` and reports its final status; `shellWrite` waits for its result and rejects with no open session | `server.test.ts` (5 tests) | behavior |
| 4 | The server emits transcript patches with removals on retry; queues send frames while busy and drains them in order; `steer` resolves correlated acks and receives patches without queueing, accepts an active provider without native steer and materializes at the continuation boundary, rejects with no open session, and a new client receives pending steers from the live session and observes their removal; `cancelPendingSteer` removes an accepted steer before materialization and is silent with no open session | `server.test.ts` (8 tests) | behavior |
| 5 | The server rejects retry/resume/compact frames while busy; each queued send resolves on its own phase cycle; `dequeueMessage`/`sendQueuedMessage`/`steerQueuedMessage`/`clearMessageQueue` behave correctly; only the active action is rejected when queued sends continue after an error; `abort` correctly reports idle vs. aborting; the server aborts the active session and disposes shell/harness resources on a close frame; a lost operation is recovered through snapshots and its original receipt; a malformed client frame is rejected at ingress; the server accepts a `ProviderResolver` and errors on an unknown session id | `server.test.ts` (12 tests) | behavior |
| 6 | Spawning a subagent returns creation immediately while the child delivers its result separately; an empty child result completes with empty output; an empty prompt fails the spawn without starting a child; a child can spawn a grandchild and the tree links correctly; `notifyParentOnIdle: false` only silences the root level; aborting a child tears down its whole subtree; `send` steers a running child and preserves its source at the continuation, or joins a busy child continuation without opening a human turn; the parent can send even with no steer verb, waking an idle root internally | `subagent.test.ts` (9 tests) | behavior |
| 7 | A sibling can show and message another sibling through the directory; lifecycle authority — send/abort reject an archived child, only resume revives it; `--no-subagents` forbids spawning while communication/reads still work; a profile with `canSpawnSubagents: false` pins its children to communication only; a child finishing after the parent went idle wakes it with an agent receipt carrying the spawning round's metadata; `notifyParentOnIdle: false` leaves an idle parent untouched when a child closes; `abortSubagents` aborts every live child without touching the parent's own turn; `demi agent abort` tears a child down after its spawn command succeeded | `subagent.test.ts` (8 tests) | behavior |
| 8 | `list` renders the tree with a self marker, `show` exposes a bounded snapshot, a finished id is absent; a profile's `systemPrompt` replaces the parent prompt, an unknown profile fails the spawn; closing the parent detaches live children, a reopen restores and finishes them; a finished child is archived (`list` shows it, `resume` revives it on its old transcript); a parent restore skips archived children while the archive stays revivable; resuming an archived child whose profile is gone fails without orphaning the archive | `subagent.test.ts` (7 tests) | behavior |
| 9 | Omitting `--profile` inherits even with declared profiles; a harness may not declare a reserved profile name; the live-children ceiling rejects a spawn beyond `MAX_LIVE_SUBAGENTS`, configurable per server; unnamed grandchildren inherit the parent's profile prompt/model, keep the agent tree after command filtering, and use the configured preview budget; an archived `--no-subagents` child stays restricted after reopening/resume; a reopened parent restores nested child checkpoints before the outer child can settle; a child lost before its first save still has its brief (queued by the create commit); a close whose wakeup was never committed is delivered once at restore, a quiescent live child closes there too; an idle root cannot reserve its running child tree, custom profiles retain product context; a tree runs while a child waits on its yield, not while a command outlives the turn; spawning succeeds while the child runs, cancelling the invoking shell does not abort the child; start-request ids survive reopening and deduplicate spawn/each resumed round; a child whose internal continuation fails closes with a failed completion receipt | `subagent.test.ts` (14 tests) | behavior |
| 10 | Stdio transport preserves `Uint8Array` fields through JSON frames; carries the same client/server frames over NDJSON, including complex action-convergence sequences; closing disposes shell foreground processes through the server; carries frames to a child-process server | `stdio-transport.test.ts` (5 tests) | behavior |
| 11 | WebSocket transports serialize frames as JSON text messages while preserving binary fields; carry client/server traffic end to end, including complex action convergence | `websocket-transport.test.ts` (3 tests) | behavior |
| 12 | The Rust runner passes the Host conformance suite over its actual wire | `host-remote/src/__tests__/runner/conformance.test.ts` | behavior (this is a cross-cutting acceptance test — it drives `AgentServer` end to end against a real runner, so it also validates H1, S1, S2 and S3; kept here because it is imported/run alongside `server.test.ts`'s scenarios and judges the server's contract) |
| 13 | A bare `AgentServer` executes correctly over a live runner; death mid-command surfaces as a tool error; reconnect resumes | `host-remote/src/__tests__/runner/remote-host-agent.test.ts` | behavior (needs H1's remote host and S2's runner connection) |
| 14 | Inherited children read and write the parent; a cross-host command preserves the child node's storage scope; a silent child runs past the HTTP idle timeout after spawn has exited | `backend/src/__tests__/scenarios/s5-subagents.test.ts` (3 tests) | behavior (backend acceptance of A3's subagent contract; needs E2/E3's HTTP layer) |

Fixtures: none beyond in-test fixtures; no golden files.

---

## A4

Owns: coding agent (harness, demi commands and their routing, todo).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The browser catalog declares every operation family with one input source per operand; browser results enforce the catalog's fields while preserving nested inspect nodes | `browser-catalog.test.ts` (2 tests) | behavior |
| 2 | The coding-agent harness exposes shell-session tools and a registered command prompt; ships no named profiles by default (omitting `--profile` inherits); the injected `demi agent` command help teaches self-contained spawn prompts; the harness leaves shell lifecycle to whoever assembles the host | `coding-harness.test.ts` (4 tests) | behavior |
| 3 | End to end, the coding agent completes a `demi`/`todo` workflow through shell-session tools; preserves workflow state across multiple user messages; preserves cwd between jobs while scoping exported variables to one job; iterates from a failing check to a passing fix; controls a long foreground command with status and abort; exercises every standard shell control tool in one flow | `coding-marathon.test.ts` (6 tests) | behavior |
| 4 | `demi file read` returns text as text, and raw bytes as `binaryStdout` at the boundary for binary files; file bodies use quoted heredocs, and body options fail before changing any file; `demi --help` documents byte-stream reads and any word stays usable as a filename; `demi file create` writes a new file from heredoc content; `demi` allows paths outside the default cwd when the namespace/Host.fs allows it; `demi file edit` replaces exact text, fails on ambiguous matches, disambiguates via context only when needed, and rejects empty old text without modifying the file; `demi file patch` applies a unified diff (with or without timestamped headers), across multiple files, creating new files, deleting via `/dev/null` targets, and validates every file before writing any of them | `demi-command.test.ts` (15 tests) | behavior |
| 5 | The todo command supports add/list/update/done with raw and JSON output; its state is isolated per agent-session id; concurrent adds keep distinct ids and both tasks; its storage survives shell recreation for the same session | `todo-command.test.ts` (4 tests) | behavior |
| 6 | End to end: `demi file create/read/edit/list` work through a real conversation | `backend/src/__tests__/scenarios/s1-files.test.ts`: "create, read, edit, list" | behavior (Gate 0 flags this as one of the Linux-hang failures — native `demi` commands through the runner; a required behavior for this WP on Linux) |
| 7 | Todos persist across turns and stay attached to their session | `backend/src/__tests__/scenarios/s4-todo.test.ts` | behavior |
| 8 | A stream past the view budget, a binary final stream, a non-zero exit with stderr, and a command outliving its observation window all report correctly | `backend/src/__tests__/scenarios/s2-output-view.test.ts` (4 tests, also listed under A1 row 13 — the tool-output contract is A1's, exercising it end to end through the coding agent is A4's) | behavior |
| 9 | A command polled to its end with `shell_status`; stdin fed with `shell_write` reaches the command; a second command is stopped with `shell_abort` while another has already ended; on a runner, the job waits for its background shell tasks | `backend/src/__tests__/scenarios/s3-long-commands.test.ts` (4 tests) | behavior (Gate 0 flags this cluster as failing on Linux — stdin/stream plumbing through `shell_write`; a required behavior for this WP) |

Fixtures: none beyond in-test tempdirs/fixture Host processes.

---

## H1

Owns: Host access, backend side (runner registry, remote host, pipes).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Concurrent token "hellos" bind only one socket, including a duplicate hello on the same socket; a socket closed during hello or during registry shutdown never becomes "online"; a claim whose socket closes rolls back the undeliverable device token; RPC terminal events abort the handler, release live stdin, and preserve the first cause; RPC authority comes from a live job on the authenticated device, and the handler receives that job's context | `backend/src/__tests__/runner-registry.test.ts` (5 tests) | behavior |
| 2 | Pairing: claim → reconnect with the device token → revoke refuses reconnect; claim codes expire and rotate on the waiting socket, a stale code is dead; the claim endpoint is rate-limited per user; a malformed runner frame closes the socket, a bad device token is rejected; a session executes on the claimed device, disconnect is a tool error, reconnect resumes (M4 acceptance); a device token holds one live connection — a newcomer is refused and can retry once the first is gone | `backend/src/__tests__/runner.test.ts` (6 tests) | behavior |
| 3 | The installer keeps backend registrations separate per install, reuses a release, and drains only its own upgrade | `backend/src/__tests__/runner-install.test.ts` | behavior |
| 4 | `--host` shell: the job runs on the named host with the right caller identity; streams stderr before stdout ends, forwards `shell_write`, and cancels the far job from both device and Cloud callers | `backend/src/__tests__/host-shell.test.ts` (2 tests) | behavior |
| 5 | Attach brings the runner identity to a Host that was created while its runner was offline; remote filesystem calls execute on the served Host preserving error codes, and an fs reply is read as the operation the caller actually asked for; remote spawn streams output, accepts stdin, reports exit; detach fails pending calls and kills in-flight spawn views, and reattach resumes; logical cwd and identity are backend-local; machine admission covers pending filesystem calls and process lifetime, then refuses cached Hosts during a transition, and a retained process holds no admission (so its machine can idle while it runs); process/shell stdin writes use bounded frames and preserve bytes before EOF, including a 65,537-byte write that must not disconnect the runner; stdin/cancellation sent during native-process startup still reach the new process; the working-tree facet lists uncommitted changes and shows the last commit; the log facet reads newest-then-cursor; jobs reuse the selected manifest only within one ordered connection | `remote-host.test.ts` (14 tests) | behavior |
| 6 | Conversation release skips offline Hosts and joins the acknowledgement without admitting new work | `conversation-release.test.ts` | behavior |
| 7 | Completion waits for publication, and later polls retain UI metadata | `edit-retention.test.ts` | behavior |
| 8 | Artifact requests require the active job and its exact manifest artifact; job completion aborts pending location resolution without a stale response | `native-artifacts.test.ts` (2 tests) | behavior |
| 9 | Remote statuses track active invocation hints independently and discard them at job exit; disconnect clears a remote job hint along with the failed job | `running-hint.test.ts` (2 tests) | behavior |
| 10 | Native commands route to their own runner keeping cwd/env and binary streams independent; a CPU-bound command can be cancelled without blocking the runner, and duplicate installation is refused; native resident services preserve output and report command failures; an active job keeps its manifest and root aliases after a manifest update | `runner/command-mode.test.ts` (4 tests) | behavior |
| 11 | A blocked job's and spawn's stdin still leave ping/filesystem/kill responsive; stdin ordering survives EOF | `runner/control.test.ts` | behavior |
| 12 | A file far over the message size limit reads and writes whole; a byte-range read works, and a missing file rejects before any byte; a reader that leaves stops the runner's read without breaking the Host; a write that cannot land leaves the destination as it was with no stray temp file; a message over the limit fails only its own request, in either direction | `runner/file-contents.test.ts` (5 tests) | behavior |
| 13 | A job runs in the runner's embedded shell, its output files live in the artifact directory; a job runs in the device environment beneath the shell; the working directory carries between jobs of a shell (explicit exit included), env does not; a job outliving its timeout is still "running", counted in the job table, takes stdin, and can be aborted; output beyond the view is a head, a gap note, and the true tail — the full stream is in the file; a dropped connection kills the job on the runner and fails it in the backend | `runner/jobs.test.ts` (6 tests) | behavior |
| 14 | Net streams: a 1 MiB payload echoes byte-equal through both pipes, input EOF half-closes, refusals carry the right code, connection loss closes the socket; a backend that fails the output pipe mid-upload ends the upload without a peer EOF | `runner/net.test.ts` (2 tests) | behavior |
| 15 | Job pipes: stdin is fetched into the job, stdout streams out as written, both report `pipe_done` | `runner/pipes.test.ts` | behavior |
| 16 | A native runner reports actual native/rpc leaf hints and clears them between shell statements | `runner/running-hint.test.ts` | behavior |
| 17 | A service stream invokes the operation with its context/cwd, carries bytes both ways, and ends with the invocation; the page going away cancels the invocation and the runner reports both pipes; the Host log keeps a stream; a one-shot call carries its arguments and learns of a failed invocation | `runner/service-streams.test.ts` (4 tests) | behavior |
| 18 | The Rust runner passes the Host conformance suite over its actual wire | `runner/conformance.test.ts` | behavior (see A3 row 12 — same test, judged there for the agent server's contract and here for the runner-registry/remote-host contract it also exercises; Gate A1's own criterion is exactly this test) |
| 19 | A cross-host RPC stays connected before its first byte and between bytes past the backend's default HTTP idle timeout | `backend/src/__tests__/scenarios/pipe-lifecycle.test.ts` | behavior (needs E2's HTTP server config) |

Fixtures: none beyond in-test fixture Host/runner processes and tempdirs.

---

## H2

Owns: shell environment and commands (command records, command ABI,
reserved names, file host store, remote shell environment, command loader
and manifests).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | `parseCommandInput` maps positionals/flags/stdin fields, validates long options and coerces numbers, handles `--json`/booleans/repeated array options, rejects unknown options/invalid values, walks nested command groups to a leaf, supports bare root leaves, treats `--help` as help at every node, and reports full paths for nested errors | `shell/src/__tests__/command.test.ts` (a large suite, ~20 tests, see file) | behavior |
| 2 | Stdin bodies cannot be supplied as options even alongside a heredoc; option values cannot swallow another option, and literal flags have explicit syntax; raw arguments use only the `--` boundary and are documented there | `command.test.ts` (3 tests) | behavior |
| 3 | Validated field values reject wire text, unknown fields, and report every validation issue; registration accepts the command-input subset of a schema and rejects the rest | `command.test.ts` (2 tests) | behavior |
| 4 | `renderCommandHelp` documents the whole tree, including enum choices, and only advertises JSON output where it is actually supported | `command.test.ts` (2 tests) | behavior |
| 5 | `CommandRegistry` registers commands and renders all prompts; rejects names reserved for shell/system commands and names unsafe as CLI path segments; rejects empty groups, handler-less leaves, and dangling field references; ambiguous input declarations are rejected at registration | `command.test.ts` (5 tests) | behavior |
| 6 | `runRegisteredCommand` implements `--help` from the same renderer, executes nested leaves and bare-leaf roots, validates JSON output when `--json` is set, and rejects invalid JSON output or JSON mode on a command without a JSON schema; cancellation during hint registration clears the hint without entering the leaf | `command.test.ts` (5 tests) | behavior |
| 7 | The reserved-name set covers every shell word, builtin and system tool; the registry rejects reserved names and accepts distinct ones | `reserved-commands.test.ts` (2 tests) | behavior |
| 8 | The file-backed host store reads/writes/lists/deletes JSON files; round-trips `Uint8Array` values inside stored JSON; keeps a concurrently overwritten key complete and parseable; rejects keys that are not relative store paths | `file-host-store.test.ts` (4 tests) | behavior |
| 9 | The root entry point exposes the Host contract and the logical cwd, and its local static closure does not import Node-only runtime source | `root-entry.test.ts` (2 tests) | behavior |
| 10 | A directory command source reads back its declaration manifest and invokes an explicit adapter | `command-loader/src/__tests__/directory-source.test.ts` | behavior |
| 11 | Manifest reconstruction preserves exclusive stdin inputs and diagnostics; a native command runs against the host filesystem with the caller's cwd/args; stdin streams into a native command with visible env and the executor's exit code; an rpc leaf runs its handler through the transport with stdin/`--json`; a group prints help, usage errors exit 1 with a reason, an unknown root exits 127; without a transport, rpc leaves report that plainly while native leaves still run; the loader exposes the manifest and the reconstructed roots; rpc arguments arrive as decoded, wholly-validated JSON | `loader.test.ts` (8 tests) | behavior |
| 12 | `buildManifest`: contradictory input sources fail both at build and after reconstruction; hashes are a pure function of content (two builds of the same tree agree, a different native descriptor changes the hash); the tree carries kinds/help/positionals/JSON Schema; duplicate roots are refused; the manifest survives a JSON round-trip; rpc/native running hints survive the wire and affect the hash | `manifest.test.ts` (`buildManifest`, 7 tests) | behavior |
| 13 | `treeFromManifest`: help rendered from the reconstructed tree equals help from the declared tree; input schemas validate identically after the round trip | `manifest.test.ts` (`treeFromManifest`, 2 tests) | behavior |
| 14 | Native manifests bind a complete, exact package and verify after JSON transport; the native catalog rejects duplicate ids, missing bindings, and missing operations | `native-manifest.test.ts` (2 tests) | behavior |
| 15 | The native package contract uses the same canonical descriptor identity as the Rust fixture; describes exactly the targets a release carries, no others; rejects repeated operations and corrupt JSON strings | `command-protocol/src/__tests__/package.test.ts` (3 tests) | behavior (needs N1's package descriptor and Rust-side fixture as the cross-language golden — see N1 row 22) |

Fixtures: the Rust-side native-package fixture referenced by row 15 and by
N1 row 22 — one golden, shared by both WPs; keep it defined once (H2 or N1,
whichever owns `internal/commandservice`'s test data) and imported by the
other.

---

## E1

Owns: backend skeleton and storage (config, lifecycle, SQLite, migrations,
stores).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | A Cloud conversation executes native programs and retains its history; conversation creation validates and reuses the client-supplied UUID; a malformed PATCH body is rejected with `400 invalid_body` | `backend/src/__tests__/backend.test.ts` (3 tests) | behavior |
| 2 | A non-numeric port stops startup, naming the offending variable; managed hosts configured without the URL guests dial also stops startup | `backend/src/__tests__/main-env.test.ts` (2 tests) | behavior |
| 3 | A work panel is saved whole, read back exactly as saved, and never interpreted by the backend | `backend/src/__tests__/panel.test.ts` | behavior |
| 4 | Maintenance postpones a resource's retirement without restarting its elapsed idle interval; new demand resets the idle interval, and no admission is reclaimed while active; disposal joins an already-unregistered retirement while unrelated resources continue; a failed cleanup releases its gate and backs off instead of looping; brief demand arriving while an asynchronous reservation waits starts a fresh idle interval | `backend/src/lifecycle/__tests__/coordinator.test.ts` (5 tests) | behavior |
| 5 | Control-plane migrations apply once and are idempotent; conversation migrations create the data-plane tables; `DbHostStore` round-trips portable JSON and lists by literal prefix; a title chosen at creation is a user title that neither a message nor a model replaces; `ControlService` conversation CRUD/ordering and device/workspace records; client conversation ids are idempotent and scoped to their owner; `DirBlobStore` content-addresses bytes and is idempotent; the tree store writes node/block rows with media stored outside the database, the cold read is the root; the tree store's contract holds end to end (brief in the create, delivery by the parent save, reopen, cascade); `host_store` scopes are isolated per conversation database; conversation handles are an LRU (a cold read holds one only until others are touched, data survives eviction); expired web sessions are swept when a session opens | `backend/src/__tests__/storage.test.ts` (13 tests) | behavior |
| 6 | Backend shutdown closes its server even when a resource owner reports a failure while releasing | `backend/src/__tests__/scenarios/backend-close.test.ts` | behavior |

Fixtures: none beyond in-test SQLite files/tempdirs.

---

## E2

Owns: HTTP API and auth (routes, sessions/accounts, product state,
installer routes).

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Accounts: the master creates admins and users, admins create only users, nobody outranks the master; the instance mode is read back as configured | `backend/src/__tests__/admin.test.ts` (2 tests) | behavior |
| 2 | Setup creates the master once and signs it in; every other route needs the cookie; login locks out after five failures, logout ends the session; the failure limiter forgets a name after the lock window (so sprayed emails do not accumulate); a user changes their own password with the current one in hand; the session slides (a request near the end renews it, silence past the end expires it); email identity is validated and case-insensitive, nickname changes persist separately; email changes require a delivered, unexpired, single-use code and survive restart; email delivery failure allows retry, five wrong codes and password changes invalidate challenges | `backend/src/__tests__/auth.test.ts` (8 tests) | behavior |
| 3 | A JSON body over its cap, or an attachment over its own cap, is refused before it is read | `backend/src/__tests__/request-bodies.test.ts` | behavior |
| 4 | State revalidation changes after settings or conversation mutations and excludes secrets; optional SPA hosting serves assets and deep-navigation routes while preserving API error responses | `backend/src/__tests__/state.test.ts` (2 tests) | behavior |
| 5 | Shared mode: the master configures the providers, everyone uses them, admins read the usage ledger by user; isolated mode: every user configures and sees only their own providers | `backend/src/__tests__/mode.test.ts` (2 tests) | behavior |
| 6 | Settings patches merge fields across a user's devices, survive restart, and stay private to that user; a reported locale is validated and stored canonically | `backend/src/__tests__/preferences.test.ts` (2 tests) | behavior |
| 7 | The matrix: another user cannot reach or act on resources that are not theirs | `backend/src/__tests__/isolation.test.ts` | behavior |
| 8 | A command context correctly names its conversation and caller, and carries the conversation's own context | `backend/src/__tests__/command-context.test.ts` | behavior |
| 9 | A page opens a user stream on its conversation; the route refuses a foreign origin, an unknown stream, an archived conversation and an offline device; an archive ends the conversation's stream | `backend/src/__tests__/user-streams.test.ts` (3 tests) | behavior |
| 10 | The capture-extension replacement scenario leaves the actual replacement to the Host | `backend/src/__tests__/browser-capture-extension.test.ts` | behavior (needs N3's browser commands and H1's Host access) |

Fixtures: none beyond in-test HTTP servers/tempdirs.

---

## E3

Owns: conversations and LLM services.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | Providers: create/list redact key material, unknown types are rejected, delete un-resolves; every provider request lands in the usage ledger and `/api/usage` aggregates it; the provider request rate limit refuses at the inference entry point; concurrent completions during a subscription login publish one provider with its vault pool; the models.dev vendor catalog carries family and a live model list, and custom endpoints name their own; a process-capable provider gets a session-scoped instance carrying the target spawn; provider edits apply only after the active request, unchanged requests retain their state, and deletion blocks inference; a process provider runs on the user's own device; a DeepSeek-vendor tool continuation replays reasoning to the compatible endpoint | `backend/src/__tests__/llm.test.ts` (9 tests) | behavior |
| 2 | Fresh memory and a database restart both avoid the upstream catalog fetch, and TTL refreshes once in the background; a failed refresh retains persisted data, cools down retries, and an explicit refresh bypasses that delay; identity changes/invalidation cancel old refreshes and prevent stale results from overwriting storage; cold failures and invalid data are explicit, and timeout/close release pending readers | `backend/src/__tests__/model-catalog-cache.test.ts` (4 tests) | behavior |
| 3 | Manually configured model parameters reach inference and a refresh never fetches an external catalog | `backend/src/__tests__/model-config.test.ts` | behavior |
| 4 | Session provider clones retain independent state and steering while refreshing edited credentials; a delayed stale vault read cannot poison the cache after a provider edit | `backend/src/__tests__/session-providers.test.ts` (2 tests) | behavior |
| 5 | An account is an encrypted record whose refreshed secret is stored only over the version it was read at; usage is kept per account and shared by every provider built for it; a pool directory from an older backend becomes account records and is removed | `backend/src/__tests__/credential-pool.test.ts` (3 tests) | behavior (backend-side wrapper around P1's credential pool contract) |
| 6 | The message-derived title is the start of the message on one line; the generated (model) title is the first non-empty line, unquoted and cut to length; the lowest thinking effort a model names is used, none for the rest | `backend/src/__tests__/conversation-title.test.ts` (3 tests) | behavior |
| 7 | The first send titles from the message, then from the model, and later sends ask for nothing further; a title request yielding nothing leaves the message-derived title; the browser repeating the placeholder at record creation settles nothing; a rename landing while a title request is in flight wins; the user can ask for a new title from any message, cut short, with the button available only while a message is newer than the title; a conversation with no message has no title to request | `backend/src/__tests__/scenarios/conversation-title.test.ts` (6 tests) | behavior |
| 8 | Malformed content/provider frames fail before mutation, and later frames still arrive; `open`/`set_provider` record only a provider the user may actually name; malformed edits never reach the session, and an archived refusal is correlated to its request; a failed storage rewrite does not poison later deliveries; attachment references are validated and resolved for sends/steers; an outbound blob-write failure is reported without dropping later frames; frame admission stays held until asynchronous handling finishes, including on failure; transcript frames carry the failure facts of the error blocks they bring, others go unchanged | `backend/src/__tests__/scoped-transport.test.ts` (8 tests) | behavior (needs A3's server frame contract) |
| 9 | Command snapshot/head/transcript-boundary roll back together on a SQL failure; cancellation during media IO prevents the command-state transaction | `backend/src/__tests__/command-state-storage.test.ts` (2 tests) | behavior (backend persistence of A1's command-state store) |
| 10 | A SQL failure after replacement rows and suffix deletion rolls back blocks/state/media/receipt together; a blob failure leaves the existing checkpoint and retained attachment bytes readable; a SIGKILL at either boundary recovers exactly one complete checkpoint without a graceful shutdown | `backend/src/__tests__/editing-storage.test.ts` (3 tests) | behavior (backend persistence of A2's editing contract) |
| 11 | A publication failure leaves a hidden complete root that recovery publishes exactly once; a Fork stays independent after its source is removed and carries no child records; a reservation with no committed root stays hidden through recovery | `backend/src/__tests__/fork-storage.test.ts` (3 tests) | behavior (backend persistence of A2's Fork contract) |
| 12 | Historical checkpoint pairs survive cold reads, Fork, and a missing side; the history route authorizes the owner and reads archived conversations without needing a live Host | `backend/src/__tests__/change-store.test.ts` (2 tests) | behavior |
| 13 | The history route sends exactly what the provider reads from each error block | `backend/src/__tests__/scenarios/failure-facts.test.ts` | behavior |
| 14 | Manual sidebar ordering/pinning and partial patches persist independently of activity; running work refuses archiving, completed output stays unread until acknowledged, archived sockets refuse writes | `backend/src/__tests__/sidebar-backend.test.ts` (2 tests) | behavior |
| 15 | Live stdin requests exactly one chunk per read and rejects unsolicited bytes | `backend/src/__tests__/live-input.test.ts` | behavior |
| 16 | A Range header asks for one part of a file, or all of it; media shows in place (images under a policy that keeps SVG inert), everything else downloads; a paced/upload body correctly follows and ends with its source, including a browser that stops reading, leaves, or a source that fails or ends abruptly; an upload body is read exactly as the write asks, and the write's own timing does not count against the browser; a browser that stalls sending an upload chunk is failed with its own reason when the transfer ends | `backend/src/__tests__/file-transfer.test.ts` (9 tests) | behavior (needs H1's remote-host pipes underneath) |
| 17 | File references preserve the exact device and path; an inaccessible reference creates no grant | `backend/src/__tests__/remote-files.test.ts` | behavior (needs H1) |
| 18 | The change routes list uncommitted changes of the execution directory and read one file; the raw routes stream a file by range under headers that keep it inert, and the committed side comes from git; archiving a conversation ends its open transfers rather than waiting for them; an upload streams into place whole, asks before overwriting a file, and never overwrites a directory; a delete takes a file or directory with its contents, refusing directories the Host itself needs | `backend/src/__tests__/working-tree.test.ts` (5 tests) | behavior (needs S3's git status/diff and H1) |
| 19 | Retained edits are call history on their target | `backend/src/__tests__/scenarios/edit-tracking.test.ts` | behavior (Gate 0 flags this cluster as failing on Linux; needs S1's edit tracking end to end) |
| 20 | Authenticated editing restores todos, preserves files, and reconciles across takeover and restart | `backend/src/__tests__/scenarios/editing.test.ts` | behavior |
| 21 | Conversation files boot Cloud, wake it after idle, and follow a target switch; attachment browsing admits only bound hosts and wakes an attached Cloud | `backend/src/__tests__/scenarios/conversation-files.test.ts` (2 tests) | behavior (needs E4's managed-Cloud wake) |
| 22 | Attaching a binary message round-trips from upload through the model to the blob route | `backend/src/__tests__/scenarios/s8-attachments.test.ts` (also listed under A1 row 14 — the upload/blob route itself is E3's) | behavior |

Fixtures: none beyond in-test SQLite/tempdirs/mock provider HTTP servers.

---

## E4

Owns: vault, managed hosts (Cloud through the machines client), expose.

| # | Behavior | Source tests | Kind |
|---|---|---|---|
| 1 | The instance secret is generated once with mode 0600, is stable across loads, and a corrupt file fails loudly; crypto round-trips, produces unique ciphertexts, and fails loudly on a wrong key or tampering; `ProviderVault` rows carry ciphertext only and CRUD round-trips typed configs; subscription uniqueness belongs to the scope+family while API-key entries stay repeatable; an ownerless provider from an older shared instance becomes the master's | `backend/src/__tests__/vault.test.ts` (5 tests) | behavior |
| 2 | Claude setup tokens stay private; accounts switch explicitly and the active account cannot be removed; adding a device-login account reserves the provider, and cancellation releases it | `backend/src/__tests__/provider-accounts.test.ts` (2 tests) | behavior |
| 3 | All six release blobs precede their immutable descriptors, and locations are freshly signed; a missing or corrupt target prevents every upload; a development release covering fewer targets is not published; the S3 adapter produces HTTPS signed GET URLs without contacting storage | `backend/src/__tests__/native-publication.test.ts` (4 tests) | behavior |
| 4 | Cloud and device target switches keep files in place and attach the departed Host; the target boundary validates discriminators, owned references, and archived-conversation switches; a compare-and-swap target switch admits exactly one winner and preserves the losing announcement | `backend/src/__tests__/switch.test.ts` (3 tests) | behavior |
| 5 | Switching Cloud → runner → Cloud keeps files with their target | `backend/src/__tests__/scenarios/s6-switch.test.ts` | behavior |
| 6 | Paired-Host release uses a one-hour idle window that resets on activity and reaches both main and attached devices; a target switch invalidates the old idle deadline; paired-Host activity resets an attached Cloud's idle deadline; a target switch, attachment detach, and archive all release before changing the binding; an hour of Cloud inactivity stops the machine without releasing the conversation | `backend/src/__tests__/scenarios/conversation-lifecycle.test.ts` (5 tests) | behavior |
| 7 | A paired device answers its log by cursor/limit/source, and 409s while offline; the Cloud answers its log while running, 409s without waking while stopped, and the log is there after waking | `backend/src/__tests__/scenarios/device-log.test.ts` (2 tests) | behavior |
| 8 | Host access recovers a Cloud that a manager restart stopped without a death event; concurrent first-use of a Cloud joins one boot, idle saves once, and the next command wakes it | `backend/src/__tests__/scenarios/s10-managed-lifecycle.test.ts` (2 tests) | behavior |
| 9 | An external reset preserves Cloud files and device identity, announces itself to the model, and is idempotent; a failed reset reports the failure and retries the same operation without losing home; a reset leaves a conversation with only Cloud attached running, and holds a Cloud-backed conversation until it ends; a reset holds a conversation on a paired device whose provider runs on the Cloud, then releases it afterward | `backend/src/__tests__/scenarios/s11-reset.test.ts` (4 tests) | behavior |
| 10 | Cloud projects share one user machine and can read each other; deleting a project preserves files | `backend/src/__tests__/scenarios/s12-cloud-workspace.test.ts` | behavior |
| 11 | R1: a backend restart while idle reconnects the runner, resumes the conversation, and carries the ledger over; R2: a mid-turn backend restart leaves no dangling tool call, and the next turn executes; R3: a runner death mid-command is a tool error, the returned runner serves the next turn, files survive; R4: Cloud persistence — files, todos and the ledger survive a backend restart | `backend/src/__tests__/scenarios/restart.test.ts` (4 tests) | behavior |
| 12 | Adding an expose on a paired device prints a URL and the relay rewrites Host/adds forwarded headers; adding on Cloud works too, and a checkpoint keeps the expose while a stop destroys it; 8 MiB bodies arrive byte-equal both ways and an event stream reaches the visitor; a WebSocket echo carries text/binary and close codes both ways; expiry destroys the record, a renewal before expiry extends it; a paired device going offline keeps its exposes and answers 502 until reconnect; another user cannot list/renew/remove, and the URL itself needs no session; removing while a connection is open ends that connection; the 65th concurrent connection answers 503 and a closed one admits again; a visitor that disappears is torn down after the idle limit and its slot released; without `DEMI_EXPOSE_DOMAIN` configured, add answers `expose_unavailable` and the state hides the feature | `backend/src/__tests__/scenarios/expose.test.ts` (11 tests) | behavior |
| 13 | `HttpResponseParser`: content-length bodies split across feeds; chunked bodies fed byte-by-byte, with extensions/trailers; interim 100-Continue responses are skipped; read-until-close bodies end with the stream; a 204 with no body finishes at the head; a serialized request head round-trips through the parser's own shape; chunk-framing helpers compose a chunked body | `backend/src/__tests__/expose-http1.test.ts` (8 tests) | behavior (HTTP/1.1-over-expose parsing, needed by row 12) |
| 14 | `exposeUrl`: a local backend on a port carries it; a public URL on the default port carries none | `backend/src/__tests__/expose-url.test.ts` (2 tests) | behavior |
| 15 | WebSocket client framing: a text frame round-trips through its own mask; 16-bit/64-bit lengths frame large payloads; a close payload carries code and reason; the handshake request names the upgrade. `WsFrameParser`: parses text/binary/ping/close frames, assembles a fragmented message, delivers frames split mid-header, rejects a masked server frame | `backend/src/__tests__/expose-ws.test.ts` (8 tests) | behavior (needed by row 12) |

Fixtures: none beyond in-test HTTP/WS servers and tempdirs.

---

## X1

Switch/cleanup checkpoint; no old tests of its own. Its acceptance is that
every ported behavior above still passes with contracts moved to Go and TS
server-side/Rust sources deleted — it re-runs the ledger, it does not add to
it.

---

## Test files not assigned to any work package, and why

All of these are excluded because they belong to the frontend, which the
task explicitly puts out of scope, not because they could not be judged:

- `packages/web/**/*.test.ts` (11 files) — frontend package, explicitly
  out of scope.
- `packages/web-ui/**/*.test.ts` (67 files) — frontend package, explicitly
  out of scope. Includes files that look backend-adjacent by name
  (`fast-mode.test.ts`, `panel-tabs.test.ts`, `work-panel.test.ts`,
  `message-editing.test.ts`, `file-browser-state.test.ts`) but that test
  UI-only state/behavior owned by `web-ui` per `docs/package-boundaries.md`.
- `packages/web-gallery/**/*.test.ts` (2 files) — frontend package,
  explicitly out of scope.
- `packages/core/src/__tests__/*.test.ts` (4 files: `attachment-tag`,
  `file-types`, `model-media`, `platform-entrypoints`) — plan.md names
  `@demicodes/core` as one of the frontend's browser-side libraries the
  frontend imports; treated as frontend-side and out of scope, even though
  its types are also consumed by backend code. If backend code actually
  needs Go equivalents of these, that is a contract-generation question for
  F4b, not a ledger behavior for any single WP — flagged as an open
  question below.
- `packages/utils/src/**/*.test.ts` (8 files, including
  `packages/utils/src/reorder.test.ts`) — same reasoning as `core`:
  plan.md names `@demicodes/utils` as frontend-side.
- `packages/browser-protocol` — plan.md names it frontend-side; it has no
  test files under the glob used here, so nothing to exclude by name, but
  noting it for completeness.

Three backend-adjacent files could not be given full behavior rows because
they use dynamically-named test functions my extraction script does not
parse as literal `test('title', ...)` calls; their content was read by hand
and is included in the ledger above rather than left unassigned:
`packages/machines/src/__tests__/home-image.test.ts` (M1 rows 4–5),
`packages/provider-codex/src/__tests__/real-codex.e2e.test.ts` (P3a row
11), `packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts`
(P4 row 13). The backend e2e files `browser-cloud.e2e.test.ts`,
`browser-live-cloud.e2e.test.ts`, `browser-live.e2e.test.ts`,
`claude-chain.e2e.test.ts`, `real-gvisor.e2e.test.ts`, and
`scenarios/browser.test.ts` are likewise dynamically named
(`acceptance(...)`, `chain(...)`, `test.skipIf(...)`) and env/hardware
gated (Cloud, real gVisor, `DEMI_BROWSER_ACCEPTANCE`); they are genuine
product-acceptance checks, not unit fixtures, so they are not converted
into ledger rows for any one WP — they should be re-run as-is (with a Go
backend/runner dropped in) as part of Gate D/C's own acceptance step
rather than ported behavior-by-behavior; each was skimmed to confirm it
does not hide a unique behavior no other test covers, and it does not
(browser-cloud/browser-live-cloud/browser-live duplicate N3/N4/E4's
covered behaviors on real Cloud/Chrome; claude-chain duplicates P4/N5's
CLI-through-vault behavior on a real runner; real-gvisor duplicates M1's
covered behaviors on a real machine; scenarios/browser duplicates N3's
element/form contract end to end).

Every other TS test file under `packages/backend`, `packages/agent`,
`packages/coding-agent`, `packages/command-loader`, `packages/command-protocol`,
`packages/host-remote`, `packages/machines`, `packages/provider*`, and
`packages/shell`, and every Rust test file under `crates/`, is assigned
above.
