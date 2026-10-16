//! What the agent can change of its own conversation
//! (`sessions-and-targets.md` § What the agent can change, § Switch the
//! primary target, § Hosts a conversation may reach): `demi host pair` asks
//! the user each time, then pairs a computer whose installer shows the code
//! and attaches it; `demi host attach` attaches a device at once, which the
//! next context block announces; `demi host move` asks once for a host the
//! conversation may not reach, checks its directory and waits, as `demi
//! host detach` does, until the conversation's work ends, when they are
//! made in one transition, and the directory decides the project it lands
//! in (`product.md` § Conversations and projects); a move that fails then
//! leaves the conversation where it was and wakes the root with Demi's
//! notice of the reason. Deleting a project archives its conversations,
//! unless one works; a device's first home report after the upgrade puts
//! the conversations the earlier release left in its directories where
//! those directories say. The model is the scripted family of the subagent
//! scenarios, the devices real runners, and the conversation holds Organize
//! Conversations and the hosts it reaches, as storage keeps them, unless a
//! test asks. No test calls a real model.

use std::sync::Arc;

use demi_shared_types::{AgentMessageEvent, Block};
use reqwest::StatusCode;
use serde_json::json;

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};

use crate::conversations::{FIRST, SECOND, THIRD, Socket, choose, create, transcript, working_on};
use rusqlite::OptionalExtension;
use crate::permissions::{decide, host_of, permissions};
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::{Harness, Paired, Session, TestBackend, eventually, stored_token};
use demi_web_api_protocol::permissions::PermissionHost;

/// Makes a project of a new directory of `device`'s home, named `name`, and
/// answers its id; the directory is `project_path(device, name)`.
pub(crate) async fn project(
    backend: &TestBackend,
    session: &Session,
    device: &Paired,
    name: &str,
) -> String {
    let path = project_path(device, name);
    std::fs::create_dir_all(&path).unwrap();
    let created = backend
        .post(
            "/api/workspaces",
            Some(session),
            json!({ "kind": "device", "deviceId": device.id(), "path": path, "name": name }),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let answer: serde_json::Value = created.json();
    answer["workspace"]["id"].as_str().unwrap().to_owned()
}

/// The directory of the project [`project`] makes on `device`.
pub(crate) fn project_path(device: &Paired, name: &str) -> String {
    format!("{}/{name}", device.runner.home())
}

/// The conversation's target as storage holds it: its kind, and the
/// workspace a project's names.
pub(crate) fn target_of(harness: &Harness, id: &str) -> (String, Option<String>) {
    harness
        .control_database()
        .query_row(
            "SELECT target_kind, target_workspace_id FROM conversations WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

/// The names of the devices attached to the conversation, as storage holds
/// them.
fn attached(harness: &Harness, id: &str) -> Vec<String> {
    harness
        .control_database()
        .prepare(
            "SELECT device.name FROM conversation_hosts host JOIN devices device ON device.id = host.device_id
             WHERE host.conversation_id = ?1 ORDER BY device.name",
        )
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Whether a move waits for the conversation, as storage holds it.
fn pending(harness: &Harness, id: &str) -> bool {
    harness
        .control_database()
        .query_row(
            "SELECT pending_id IS NOT NULL FROM conversations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

/// Grants the conversation Organize Conversations, as an allow stores it.
fn grant_organize(harness: &Harness, id: &str) {
    harness
        .control_database()
        .execute(
            "INSERT INTO permission_grants (conversation_id, category, granted_at) VALUES (?1, 'conversation.organize', 1)",
            [id],
        )
        .unwrap();
}

/// Lets the conversation reach `device`, as an allow of it stores it.
fn reach(harness: &Harness, id: &str, device: &Paired) {
    harness
        .control_database()
        .execute(
            "INSERT INTO reachable_hosts (conversation_id, device_id, added_at) VALUES (?1, ?2, 1)",
            [id, device.id()],
        )
        .unwrap();
}

/// The conversation's primary device and directory, as storage holds them.
fn primary(harness: &Harness, id: &str) -> (Option<String>, Option<String>) {
    harness
        .control_database()
        .query_row(
            "SELECT target_device_id, target_path FROM conversations WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

/// What the root of `id` was asked last.
fn last_asked(scripts: &Scripts, id: &str) -> String {
    scripts
        .asked(|session| session == id)
        .pop()
        .expect("the root was asked")
}

// Several seconds: two real devices, and one runs the shell jobs of a turn.
#[tokio::test]
async fn an_attach_applies_at_once_and_a_detach_and_a_move_wait_until_the_work_ends() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, _, _) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    let notes = project(&backend, &master, &laptop, "notes").await;
    grant_organize(&harness, FIRST);
    reach(&harness, FIRST, &studio);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi host attach studio"),
            shell("t2", &format!("demi host detach studio && demi host move laptop {}", project_path(&laptop, "notes"))),
            shell("t3", "demi host list && demi host current"),
            say("done"),
        ],
    );
    socket.chat("m1", "Use the studio, then tidy up").await;
    let asked = scripts.asked(|session| session == FIRST);
    // The attach applied at once, and the next request's context block
    // announced it.
    let after_attach = &asked[1];
    assert!(after_attach.contains("Attached studio as studio"), "{after_attach}");
    assert!(after_attach.contains("[Attached hosts changed]"), "{after_attach}");
    // The detach and the move waited: studio was still attached, and the
    // conversation still ran in its directory, for the turn's next command.
    let after_waits = &asked[3];
    assert!(
        after_waits.contains("studio is detached when this conversation's work ends"),
        "{after_waits}"
    );
    assert!(
        after_waits.contains(&format!("The move to laptop ({}) applies", project_path(&laptop, "notes"))),
        "{after_waits}"
    );
    assert!(after_waits.contains("(attached)"), "{after_waits}");
    // Nor did the turn's next request learn of a change: the attach's block
    // is the last it holds.
    assert!(after_waits.contains("[Execution context 1]"), "{after_waits}");
    assert!(!after_waits.contains("[Execution context 2]"), "{after_waits}");
    assert!(after_waits.contains("host: workspace \\\"work\\\""), "{after_waits}");

    // Once the turn ended both were made, in one transition.
    eventually("the move and the detach are made", || async {
        target_of(&harness, FIRST) == ("workspace".to_owned(), Some(notes.clone()))
            && attached(&harness, FIRST).is_empty()
    })
    .await;
    assert!(!pending(&harness, FIRST));
    backend.close().await;
}

// Several seconds: two real runners, one of them paired by the agent, which
// runs the shell jobs of two turns, the second woken by the user's allow.
#[tokio::test]
async fn an_agent_pairs_a_computer_from_its_installers_code_once_the_user_allows_that_code() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, _, provider) = tree_on(&scripts, Harness::new()).await;
    let state = backend.sync(&master).await.snapshot().await;
    // The computer the agent installed the runner on, which waits to be
    // paired under the code it shows.
    let computer = RunnerProcess::start(
        &backend.url,
        RunnerProcessOptions {
            name: "buildbox".into(),
            ..RunnerProcessOptions::default()
        },
    );
    let code = computer.pairing_code(0).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // A pairing asks the user, every time.
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi host install-command --system linux"),
            shell("t2", &format!("demi host pair {code}; echo exit=$?")),
            say("I asked the user"),
        ],
    );
    socket.chat("m1", "Pair the build computer").await;
    let asked = scripts.asked(|session| session == FIRST);
    // The command the agent runs there is the one Add Device shows.
    assert!(asked[1].contains(&state.install_commands.linux), "{}", asked[1]);
    assert!(
        asked[2].contains("this conversation needs the user's permission to pair a computer and reach it; the request was sent to the user"),
        "{}",
        asked[2]
    );
    assert!(asked[2].contains("exit=1"), "{}", asked[2]);
    let requests = permissions(&backend, &master, FIRST).await;
    assert_eq!(requests.requests.len(), 1, "{requests:?}");
    let request = &requests.requests[0];
    assert_eq!(host_of(request), Some(&PermissionHost::Pairing {}));

    // The allow lets that code pair once: the computer is paired, attached
    // under its device's name and reached; the same code again, and another
    // code, ask again.
    scripts.root(
        FIRST,
        vec![
            shell("t3", &format!("demi host pair {code} --name build")),
            shell("t4", &format!("demi host pair {code}")),
            shell("t5", "demi host shell --host buildbox 'echo reached'"),
            shell("t6", "demi host pair 0000-0000-0000-0000-0000-0000-00; echo exit=$?"),
            shell("t7", &format!("demi host pair {code}; echo exit=$?")),
            say("done"),
        ],
    );
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    eventually("the woken turn ends", || async {
        scripts.asked(|session| session == FIRST).len() == 9
    })
    .await;
    let asked = scripts.asked(|session| session == FIRST);
    assert!(
        asked[3].contains("The user allowed this conversation to pair a computer and reach it"),
        "{}",
        asked[3]
    );
    // No agent names the host: `--name` is no option of the pair.
    assert!(asked[4].contains("unexpected argument '--name'"), "{}", asked[4]);
    assert!(
        asked[5].contains("Paired as buildbox; attached as buildbox."),
        "{}",
        asked[5]
    );
    assert!(asked[6].contains("reached"), "{}", asked[6]);
    // Each asks again: the result of its own call says so.
    for (asked, step) in [(&asked[7], "t6"), (&asked[8], "t7")] {
        let (_, result) = asked
            .rsplit_once(&format!("ToolResult {{ tool_use_id: \"{step}\""))
            .expect("the step's result");
        assert!(
            result.contains("permission to pair a computer and reach it; the request") && result.contains("exit=1"),
            "{result}"
        );
    }

    // A code allowed in one conversation pairs nothing in another.
    harness
        .control_database()
        .execute(
            "INSERT INTO pairing_passes (conversation_id, code, allowed_at) VALUES (?1, 'K7QM-2XWD', 1)",
            [FIRST],
        )
        .unwrap();
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    working_on(&harness, &laptop, SECOND);
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;
    scripts.root(SECOND, vec![shell("s1", "demi host pair K7QM-2XWD; echo exit=$?"), say("asked")]);
    second.chat("n1", "Pair it here").await;
    assert!(
        last_asked(&scripts, SECOND).contains("permission to pair a computer and reach it; the request"),
        "{}",
        last_asked(&scripts, SECOND)
    );
    // It is the user's device, its runner holds its token, and the
    // conversation reaches it.
    let devices = backend.devices(&master).await;
    assert!(
        devices.iter().any(|device| device.name == "buildbox"),
        "{devices:?}"
    );
    assert!(!stored_token(&computer).await.is_empty());
    assert_eq!(attached(&harness, FIRST), ["buildbox"]);
    backend.close().await;
}

// Several seconds: two real devices, and one runs the shell jobs of two
// turns.
#[tokio::test]
async fn a_move_that_fails_leaves_the_conversation_in_place_and_wakes_the_root_with_the_reason() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, root, _) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    let site = format!("{}/site", studio.runner.home());
    std::fs::create_dir_all(&site).unwrap();
    reach(&harness, FIRST, &studio);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The device is revoked while the turn that asked for the move runs.
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi host move studio {site}; {WAIT}")),
            say("asked"),
        ],
    );
    scripts.root(FIRST, vec![say("noted")]);
    socket
        .send(&crate::conversations::send("m1", "Move to the studio"))
        .await;
    eventually("the move waits", || async { pending(&harness, FIRST) }).await;
    let revoked = backend
        .delete(&format!("/api/devices/{}", studio.id()), &master)
        .await;
    assert_eq!(revoked.status, StatusCode::OK);
    std::fs::write(format!("{root}/go"), "").unwrap();
    eventually("the root is told", || async {
        last_asked(&scripts, FIRST).contains("Demi could not make the move")
    })
    .await;
    let told = last_asked(&scripts, FIRST);
    assert!(
        told.contains(&format!(
            "Demi could not make the move this conversation's agent asked for: the device is no longer paired. The conversation still runs on laptop ({root})."
        )),
        "{told}"
    );
    assert!(told.contains("\\\"sender\\\":\\\"demi\\\""), "{told}");
    socket.until_idle().await;
    assert_eq!(target_of(&harness, FIRST).0, "workspace");
    assert!(!pending(&harness, FIRST));
    let history = transcript(&backend, &master, FIRST).await;
    let notice = history
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::AgentMessage(message) => Some(&message.message),
            _ => None,
        })
        .expect("the notice is in the transcript");
    assert!(notice.id.as_str().starts_with("move-failed:"), "{notice:?}");
    // The receipt row says why after Move failed, worded by the page from
    // the refusal's code; the model reads the reason in the message.
    assert!(
        matches!(
            &notice.event,
            AgentMessageEvent::MoveFailed { reason, code }
                if notice.content.contains(reason.as_str())
                    && code.as_deref() == Some("device_not_found")
        ),
        "{notice:?}"
    );
    assert_eq!(notice.sender, None);
    backend.close().await;
}

/// The workspace selection of `id` as storage holds it: the project's
/// name and directory; none outside a project.
fn project_of(harness: &Harness, id: &str) -> Option<(String, String)> {
    harness
        .control_database()
        .query_row(
            "SELECT workspace.name, workspace.path FROM conversations conversation
               JOIN workspaces workspace ON workspace.id = conversation.target_workspace_id
             WHERE conversation.id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

/// Where a conversation runs decides its project (`product.md`
/// § Conversations and projects): `demi host move` to a directory puts the
/// conversation in a new project named after it, which a second
/// conversation moved there joins; a project's subdirectory is a project of
/// its own; a second directory of the same name takes *(2)*; the Host's
/// home puts it among Conversations.
/// Planted defect this catches: a move that keeps the conversation outside
/// projects in a directory that is none's home, or that makes a second
/// project of a directory that has one.
// Several seconds: a real device runs the shell jobs of six turns of two
// conversations.
#[tokio::test]
async fn a_host_move_puts_the_conversation_in_its_directorys_project_and_the_home_among_conversations() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, _, provider) = tree_on(&scripts, Harness::new()).await;
    let home = laptop.runner.home().to_owned();
    for directory in ["site/packages", "other/site"] {
        std::fs::create_dir_all(format!("{home}/{directory}")).unwrap();
    }
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    working_on(&harness, &laptop, SECOND);
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;
    let moves = [
        (FIRST, "site", Some(("site", "site"))),
        (SECOND, "site", Some(("site", "site"))),
        (SECOND, "other/site", Some(("site (2)", "other/site"))),
        (FIRST, "site/packages", Some(("packages", "site/packages"))),
        (FIRST, "", None),
    ];
    let mut site = None;
    for (turn, (id, directory, landed)) in moves.into_iter().enumerate() {
        let socket = if id == FIRST { &mut first } else { &mut second };
        let path = if directory.is_empty() { home.clone() } else { format!("{home}/{directory}") };
        scripts.root(id, vec![shell("t", &format!("demi host move laptop {path}")), say("moved")]);
        socket.chat(&format!("m{turn}"), "Move").await;
        let expected = landed.map(|(name, directory)| (name.to_owned(), format!("{home}/{directory}")));
        eventually("the move is made", || async { project_of(&harness, id) == expected && !pending(&harness, id) }).await;
        // The second conversation moved to `site` joins the first's project.
        let workspace = target_of(&harness, id).1;
        if directory == "site" {
            assert_eq!(*site.get_or_insert(workspace.clone()), workspace, "{id}");
        }
    }
    assert_eq!(target_of(&harness, FIRST), ("home".to_owned(), None));
    let summary = crate::conversations::summary(&backend, &master, FIRST).await;
    assert_eq!(
        (serde_json::to_value(&summary.target).unwrap(), summary.cwd.as_str()),
        (json!({ "kind": "device", "deviceId": laptop.id(), "path": home }), home.as_str())
    );
    backend.close().await;
}

/// Deleting a project whose conversation works is refused as its archive
/// is, and nothing changes; once the work ends the deletion archives it.
/// Planted defect this catches: a deletion that archives a conversation
/// under the agent that runs in it.
// Several seconds: a real device runs a turn's shell job, which waits.
#[tokio::test]
async fn a_project_whose_conversation_works_is_not_deleted() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, root, _) = tree_on(&scripts, Harness::new()).await;
    let (_, project) = target_of(&harness, FIRST);
    let project = project.expect("the conversation works in its directory's project");
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(FIRST, vec![shell("t1", WAIT), say("done")]);
    socket
        .send(&crate::conversations::send("m1", "Wait"))
        .await;
    eventually("the command runs", || async { !scripts.asked(|session| session == FIRST).is_empty() }).await;
    let refused = backend
        .delete(&format!("/api/workspaces/{project}"), &master)
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, demi_web_api_protocol::error::ErrorCode::TurnInFlight)
    );
    assert_eq!(target_of(&harness, FIRST), ("workspace".to_owned(), Some(project.clone())));
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    let deleted = backend
        .delete(&format!("/api/workspaces/{project}"), &master)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let archived: bool = harness
        .control_database()
        .query_row("SELECT archived FROM conversations WHERE id = ?1", [FIRST], |row| row.get(0))
        .unwrap();
    assert!(archived);
    backend.close().await;
}

/// What a release before projects by directory left on a paired device
/// settles when the device's runner first reports its home after the
/// upgrade, by the directories its runner resolves (`upgrades.md` § What
/// crosses releases): two workspaces of one directory, `site/` and a link
/// to it, become the older, with the newer's conversation; a workspace at
/// the home, reached through a link, goes and its conversation runs in the
/// home; and each conversation outside a project moves to the home or its
/// directory's project, a directory gone as it was spelled. A device
/// revoked before its first report keeps its conversations outside
/// projects, in their directories.
/// Planted defect this catches: projects compared by spelling, so one
/// directory stays two projects or the home a project; and conversations
/// resolved before the runner names their directories.
// About three seconds: one paired runner connects twice, another once.
#[tokio::test]
async fn a_devices_first_home_report_settles_what_an_upgrade_left_by_the_directories_its_runner_names() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let mut desk = backend.pair(&master, "desk").await;
    let home = laptop.runner.home().to_owned();
    std::fs::create_dir(format!("{home}/site")).unwrap();
    std::os::unix::fs::symlink(format!("{home}/site"), format!("{home}/link-to-site")).unwrap();
    std::os::unix::fs::symlink(&home, format!("{home}/me")).unwrap();
    const FOURTH: &str = "3c2b1a0f-4e9d-4c1e-9d2b-7a1c2e3f4a5b";
    const FIFTH: &str = "2b1a0f3c-4e9d-4c1e-9d2b-7a1c2e3f4a5b";
    const SIXTH: &str = "1a0f3c2b-4e9d-4c1e-9d2b-7a1c2e3f4a5b";
    for id in [FIRST, SECOND, THIRD, FOURTH, FIFTH, SIXTH] {
        create(&backend, &master, id).await;
    }
    laptop.runner.stop().await;
    desk.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;
    backend.until_online(&master, desk.id(), false).await;
    // As the migration leaves them: each home unknown, the workspaces as
    // an earlier release named them, each conversation in a directory of
    // its device or one of those workspaces.
    let database = harness.control_database();
    database.execute("UPDATE devices SET home = NULL", []).unwrap();
    let user: String = database.query_row("SELECT user_id FROM devices WHERE id = ?1", [laptop.id()], |row| row.get(0)).unwrap();
    for (id, path, created) in [("w-site", format!("{home}/site/"), 1), ("w-link", format!("{home}/link-to-site"), 2), ("w-me", format!("{home}/me"), 3)] {
        database
            .execute(
                "INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at) VALUES (?1, ?2, ?3, ?4, ?1, ?5, ?5)",
                rusqlite::params![id, user, laptop.id(), path, created],
            )
            .unwrap();
    }
    let left = [
        (FIRST, laptop.id(), "device", format!("{home}/me/")),
        (SECOND, laptop.id(), "device", format!("{home}/link-to-site")),
        (THIRD, laptop.id(), "device", format!("{home}/gone/")),
        (FOURTH, laptop.id(), "w-link", String::new()),
        (FIFTH, laptop.id(), "w-me", String::new()),
        (SIXTH, desk.id(), "device", "/srv/app".to_owned()),
    ];
    for (id, device, kind, path) in &left {
        let (kind, path, workspace) = match *kind {
            "device" => ("device", Some(path.clone()), None),
            workspace => ("workspace", database.query_row("SELECT path FROM workspaces WHERE id = ?1", [workspace], |row| row.get(0)).ok(), Some(workspace)),
        };
        database
            .execute(
                "UPDATE conversations SET target_kind = ?2, target_device_id = ?3, target_path = ?4,
                   target_workspace_id = ?5 WHERE id = ?1",
                rusqlite::params![id, kind, device, path, workspace],
            )
            .unwrap();
    }
    // The desk is revoked before its runner ever reports a home.
    let revoked = backend.delete(&format!("/api/devices/{}", desk.id()), &master).await;
    assert!(revoked.status.is_success(), "{}", revoked.status);
    // Until its runner reports, nothing moves.
    assert_eq!(target_of(&harness, FIRST).0, "device");
    laptop.runner.start_again();
    eventually("the device's first home is settled", || async {
        target_of(&harness, FIRST).0 == "home"
    })
    .await;
    let site = Some(("w-site".to_owned(), format!("{home}/site")));
    assert_eq!(project_of(&harness, SECOND), site);
    assert_eq!(project_of(&harness, FOURTH), site);
    assert_eq!(target_of(&harness, FIFTH), ("home".to_owned(), None));
    assert_eq!(project_of(&harness, THIRD), Some(("gone".to_owned(), format!("{home}/gone"))));
    let workspaces: Vec<String> = database
        .prepare("SELECT id FROM workspaces WHERE id LIKE 'w-%' ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(workspaces, ["w-site"]);
    // Each conversation of the project names its directory as the runner does.
    let paths: Vec<String> = database
        .prepare("SELECT DISTINCT target_path FROM conversations WHERE target_workspace_id = 'w-site'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(paths, [format!("{home}/site")]);
    let sixth: (String, Option<String>) = database
        .query_row("SELECT target_kind, target_path FROM conversations WHERE id = ?1", [SIXTH], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(sixth, ("device".to_owned(), Some("/srv/app".to_owned())));
    backend.close().await;
}

// Several seconds: two real devices, and one runs the shell jobs of five
// turns, the second woken by the user's allow.
#[tokio::test]
async fn a_host_move_asks_once_for_a_host_out_of_reach_and_lands_in_its_directory_with_the_host_it_left_attached() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, root, _) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    let app = format!("{}/app", studio.runner.home());
    std::fs::create_dir_all(&app).unwrap();
    // A project on the studio named as the host the conversation runs on.
    project(&backend, &master, &studio, "laptop").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Listing the devices asks nothing; a move names a host, never a
    // project, so `laptop` is the conversation's own; moving to a host the
    // conversation may not reach asks to reach it.
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi host devices"),
            shell("t1b", &format!("demi host move laptop {root}")),
            shell("t2", &format!("demi host move studio {app}; echo exit=$?")),
            say("I asked the user"),
        ],
    );
    socket.chat("m1", "Work on the studio").await;
    let asked = scripts.asked(|session| session == FIRST);
    assert!(asked[1].contains(&format!("studio  {}  online", studio.id())), "{}", asked[1]);
    assert!(asked[2].contains("This conversation runs there already."), "{}", asked[2]);
    assert!(
        asked[3].contains("demi host move: this conversation needs the user's permission to reach studio; the request was sent to the user"),
        "{}",
        asked[3]
    );
    let requests = permissions(&backend, &master, FIRST).await;
    assert_eq!(requests.requests.len(), 1, "{requests:?}");
    let request = &requests.requests[0];
    assert_eq!(
        host_of(request),
        Some(&PermissionHost::Device {
            device_id: studio.id().try_into().unwrap(),
            name: "studio".into()
        })
    );

    // Once allowed, a directory that is none there fails at once with
    // nothing pending, and the move to one waits until the work ends.
    scripts.root(
        FIRST,
        vec![
            shell("t3", &format!("demi host move studio {app}/missing; echo exit=$?")),
            shell("t4", &format!("demi host move studio {app}")),
            say("moving"),
        ],
    );
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    eventually("the move is made", || async {
        primary(&harness, FIRST) == (Some(studio.id().to_owned()), Some(app.clone()))
    })
    .await;
    let asked = scripts.asked(|session| session == FIRST);
    assert!(
        asked[5].contains(&format!("{app}/missing is not a directory on studio")) && asked[5].contains("exit=1"),
        "{}",
        asked[5]
    );
    assert!(
        asked[6].contains(&format!("The move to studio ({app}) applies when this conversation's work ends")),
        "{}",
        asked[6]
    );
    assert_eq!(attached(&harness, FIRST), ["laptop"]);
    assert!(!pending(&harness, FIRST));

    // The host it left and the one it was allowed stay reachable: moving
    // back, detaching studio and attaching it again ask nothing.
    scripts.root(FIRST, vec![shell("t5", &format!("demi host move laptop {root}")), say("back")]);
    socket.chat("m2", "Go back").await;
    eventually("the move back is made", || async {
        primary(&harness, FIRST) == (Some(laptop.id().to_owned()), Some(root.clone()))
    })
    .await;
    scripts.root(FIRST, vec![shell("t6", "demi host detach studio"), say("detached")]);
    socket.chat("m3", "Let go of the studio").await;
    eventually("the detach is made", || async { attached(&harness, FIRST).is_empty() }).await;
    scripts.root(FIRST, vec![shell("t7", "demi host attach studio"), say("attached")]);
    socket.chat("m4", "Use the studio again").await;
    eventually("the attach ran", || async {
        last_asked(&scripts, FIRST).contains("Attached studio as studio")
    })
    .await;
    assert!(permissions(&backend, &master, FIRST).await.requests.is_empty());
    backend.close().await;
}

// Several seconds: the backend starts twice over the same data, and the
// project's device is a runner that has not reconnected.
#[tokio::test]
async fn a_move_a_restart_cut_off_is_made_at_the_next_start_even_to_an_offline_device() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, _, _) = tree_on(&scripts, Harness::new()).await;
    let mut studio = backend.pair(&master, "studio").await;
    let website = project(&backend, &master, &studio, "website").await;
    studio.runner.stop().await;
    let address = backend.address();
    backend.close().await;

    // The command recorded the move, and the backend stopped before the
    // tree was idle: the move as storage holds it.
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET pending_id = 'm1', pending_kind = 'workspace', pending_workspace_id = ?2
             WHERE id = ?1",
            [FIRST, website.as_str()],
        )
        .unwrap();
    let backend = harness.start_at(address).await;
    eventually("the move is made", || async {
        target_of(&harness, FIRST) == ("workspace".to_owned(), Some(website.clone()))
    })
    .await;
    assert!(!pending(&harness, FIRST));
    backend.close().await;
}

// Several seconds: two real devices, a parent and its child on one, a second
// conversation on the other, and the child's shell waits across the rename.
#[tokio::test]
async fn a_device_rename_reaches_each_conversation_and_node_that_reaches_the_device_once() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, root, provider) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    harness.attach(FIRST, studio.id());
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    working_on(&harness, &studio, SECOND);
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;

    // Each node reads the context once before the rename: the conversation
    // on the studio, the parent, and its child, whose shell then waits.
    scripts.root(SECOND, vec![say("hello")]);
    second.chat("s1", "Hello").await;
    scripts.child(vec![shell("c1", &format!("{WAIT}; demi host list")), say("listed")]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --description lister <<< 'List the hosts'"),
            say("dispatched"),
            // The child's completion wakes the idle parent.
            say("received"),
        ],
    );
    first.chat("m1", "Delegate the listing").await;
    let child = |session: &str| session != FIRST && session != SECOND;
    eventually("the child asked", || async { !scripts.asked(child).is_empty() }).await;

    let renamed = backend
        .patch(&format!("/api/devices/{}", studio.id()), &master, json!({ "name": "Build Box" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    std::fs::write(format!("{root}/go"), "").unwrap();
    first.until_idle().await;
    scripts.root(FIRST, vec![say("again")]);
    first.chat("m2", "Anything new?").await;
    scripts.root(SECOND, vec![say("again")]);
    second.chat("s2", "Anything new?").await;
    scripts.root(SECOND, vec![say("still")]);
    second.chat("s3", "And now?").await;

    // Where the device is attached, each node's history holds one block
    // that names it anew and says the attached hosts changed, and no
    // switch: the child's for its request after its shell, the parent's
    // for its wake, which its next turn does not repeat.
    let attached = format!("Attached hosts: \\\"Build Box\\\" (host {}, ", studio.id());
    let parent = scripts.asked(|session| session == FIRST);
    let children = scripts.asked(child);
    for latest in [parent.last().unwrap(), children.last().unwrap()] {
        assert_eq!(latest.matches(&attached).count(), 1, "{latest}");
        assert_eq!(latest.matches("[Attached hosts changed]").count(), 1, "{latest}");
        assert!(!latest.contains("[Execution target switched]"), "{latest}");
    }
    assert!(!children[0].contains("Build Box"), "{}", children[0]);
    // Where it is the primary Host, the next block names it anew, once.
    let on_studio = scripts.asked(|session| session == SECOND);
    assert!(!on_studio[0].contains("Build Box"), "{}", on_studio[0]);
    let latest = on_studio.last().unwrap();
    assert_eq!(latest.matches("Primary host: Build Box").count(), 1, "{latest}");
    assert_eq!(latest.matches("[Execution context ").count(), 2, "{latest}");
    backend.close().await;
}
