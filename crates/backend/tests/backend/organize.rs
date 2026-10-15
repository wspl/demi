//! What the agent can change of its own conversation
//! (`sessions-and-targets.md` § What the agent can change, § Switch the
//! primary target, § Hosts a conversation may reach): `demi host pair` asks
//! the user each time, then pairs a computer whose installer shows the code
//! and attaches it; `demi host attach` attaches a device at once, which the
//! next context block announces; `demi host move` asks once for a host the
//! conversation may not reach, checks its directory and waits, as `demi
//! host detach` and `demi conversation move` do, until the conversation's
//! work ends, when they are made in one transition; a move that fails then
//! leaves the conversation where it was and wakes the root with Demi's
//! notice of the reason. The model is the scripted family of the subagent
//! scenarios, the devices real runners, and the conversation holds Organize
//! Conversations and the hosts it reaches, as storage keeps them, unless a
//! test asks. No test calls a real model.

use std::sync::Arc;

use demi_shared_types::{AgentMessageEvent, Block};
use reqwest::StatusCode;
use serde_json::json;

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};

use crate::conversations::{FIRST, SECOND, Socket, choose, create, transcript, working_on};
use crate::permissions::{decide, host_of, permissions};
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::{Harness, Paired, Session, TestBackend, eventually, stored_token};
use demi_web_api_protocol::permissions::PermissionHost;

/// Makes a project of a new directory of `device`'s home, named `name`, and
/// answers its id.
pub(crate) async fn project(
    backend: &TestBackend,
    session: &Session,
    device: &Paired,
    name: &str,
) -> String {
    let path = format!("{}/{name}", device.runner.home());
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
            shell("t2", "demi host detach studio && demi conversation move notes"),
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
        after_waits.contains("The move into notes (laptop, "),
        "{after_waits}"
    );
    assert!(after_waits.contains("(attached)"), "{after_waits}");
    // Nor did the turn's next request learn of a change: the attach's block
    // is the last it holds.
    assert!(after_waits.contains("[Execution context 1]"), "{after_waits}");
    assert!(!after_waits.contains("[Execution context 2]"), "{after_waits}");
    assert!(after_waits.contains("host: machine \\\"laptop\\\""), "{after_waits}");

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

// Several seconds: a real device runs the shell jobs of two turns.
#[tokio::test]
async fn a_move_that_fails_leaves_the_conversation_in_place_and_wakes_the_root_with_the_reason() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, root, _) = tree_on(&scripts, Harness::new()).await;
    let shop = project(&backend, &master, &laptop, "shop-app").await;
    grant_organize(&harness, FIRST);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The project is deleted while the turn that asked for the move runs.
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi conversation move shop-app; {WAIT}")),
            say("asked"),
        ],
    );
    scripts.root(FIRST, vec![say("noted")]);
    socket
        .send(&crate::conversations::send("m1", "Move into shop-app"))
        .await;
    eventually("the move waits", || async { pending(&harness, FIRST) }).await;
    let deleted = backend
        .delete(&format!("/api/workspaces/{shop}"), &master)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    std::fs::write(format!("{root}/go"), "").unwrap();
    eventually("the root is told", || async {
        last_asked(&scripts, FIRST).contains("Demi could not make the move")
    })
    .await;
    let told = last_asked(&scripts, FIRST);
    assert!(
        told.contains(&format!(
            "Demi could not make the move this conversation's agent asked for: the project no longer exists. The conversation still runs on laptop ({root})."
        )),
        "{told}"
    );
    assert!(told.contains("\\\"sender\\\":\\\"demi\\\""), "{told}");
    socket.until_idle().await;
    assert_eq!(target_of(&harness, FIRST).0, "device");
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
                    && code.as_deref() == Some("workspace_not_found")
        ),
        "{notice:?}"
    );
    assert_eq!(notice.sender, None);
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
    let shop = project(&backend, &master, &studio, "shop-app").await;
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
            [FIRST, shop.as_str()],
        )
        .unwrap();
    let backend = harness.start_at(address).await;
    eventually("the move is made", || async {
        target_of(&harness, FIRST) == ("workspace".to_owned(), Some(shop.clone()))
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
