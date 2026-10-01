//! The host access's operations over a shard (`backend-host-access`): the
//! `demi host` group, a node's jobs and the remote files a message refers
//! to, each against a runner the test plays.

mod host_commands {
    use std::collections::BTreeMap;
    use std::rc::{Rc, Weak};

    use demi_backend_host_access::HostShard;
    use demi_backend_host_access::host_commands::host_group;
    use demi_command_protocol::{CommandCaller, CommandContext};
    use demi_shell::testing::MemoryPort;
    use demi_shell::{CommandSet, GroupBuilder, RpcInvocation};
    use demi_web_api::ids::{ConversationId, DeviceId, UserId};
    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use demi_backend_runners::command_context::default_locale;
    use demi_backend_storage::accounts::TokenHash;
    use demi_backend_storage::control::testing;
    use demi_backend_storage::conversation_index::{AttachedHostRecord, Creation, RecordChange};
    use demi_runner_protocol::wire::RunnerPlatform;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    fn invocation(leaf: &str, args: Value) -> RpcInvocation {
        RpcInvocation {
            path: vec!["demi".into(), "host".into(), leaf.into()],
            argv: Vec::new(),
            args: args.as_object().unwrap().clone(),
            json: false,
            cwd: "/work".into(),
            env: BTreeMap::new(),
            context: CommandContext {
                conversation: ID.into(),
                caller: CommandCaller::agent(1),
                locale: default_locale(),
            },
            caller: Some(demi_shell::JobCaller {
                node: demi_core::NodeId::try_from("node-1").unwrap(),
                generation: 0,
            }),
            stdin: false,
            pipes: None,
        }
    }

    #[tokio::test(flavor = "local")]
    async fn the_group_names_the_hosts_the_conversation_reaches_and_refuses_one_it_does_not() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let device = |name: &'static str| {
            let control = control.clone();
            let owner = owner.clone();
            async move {
                control
                    .create_device(
                        owner,
                        name.into(),
                        RunnerPlatform::Linux,
                        TokenHash::of(name),
                    )
                    .await
                    .unwrap()
                    .id
            }
        };
        let laptop: DeviceId = device("laptop").await;
        let ci: DeviceId = device("ci").await;
        testing::execute(
            &control,
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?2, target_path = '/work'
             WHERE id = ?1",
            vec![ID.to_owned(), laptop.to_string()],
        )
        .await;
        let attached = AttachedHostRecord {
            device: ci.clone(),
            name: "ci".into(),
            cwd: None,
        };
        control
            .change_conversation(id.clone(), RecordChange::Attach(attached))
            .await
            .unwrap();
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let _link = shard.connect_for_tests(&laptop, "/home/ana");
                let mut commands = CommandSet::new();
                let this: Rc<dyn HostShard> = shard.clone();
                let hosts: Weak<dyn HostShard> = Rc::downgrade(&this);
                commands
                    .register(GroupBuilder::new("demi", "Demi.").group(host_group(hosts)))
                    .unwrap();
                let run = async |leaf: &str, args: Value| {
                    let port = MemoryPort::new();
                    let code = commands
                        .dispatch(invocation(leaf, args), port.port(CancellationToken::new()))
                        .await
                        .unwrap();
                    let stdout = String::from_utf8(port.stdout()).unwrap();
                    let stderr = String::from_utf8(port.stderr()).unwrap();
                    (code, stdout, stderr)
                };
                vec![
                    run("list", json!({})).await,
                    run("current", json!({})).await,
                    run("shell", json!({ "host": "elsewhere", "script": "pwd" })).await,
                    run("shell", json!({ "host": "ci", "script": "  " })).await,
                    // A caller that is no job on a device has no pipes to
                    // hand over.
                    run("shell", json!({ "host": "ci", "script": "pwd" })).await,
                    (0, laptop.to_string(), ci.to_string()),
                ]
            })
            .await
            .unwrap();
        let (_, laptop, ci) = &answers[5];
        assert_eq!(
            answers[0],
            (
                0,
                format!(
                    "laptop  {laptop}  online  /work  (main)\nci  {ci}  offline  ?  (attached)\n"
                ),
                String::new()
            )
        );
        assert_eq!(
            answers[1],
            (
                0,
                format!("host: machine \"laptop\" ({laptop}, online) — /work\n"),
                String::new()
            )
        );
        assert_eq!(
            answers[2],
            (
                1,
                String::new(),
                "host shell: host elsewhere is not reachable from this conversation (see `demi host list`)\n".into()
            )
        );
        assert_eq!(
            answers[3],
            (
                2,
                String::new(),
                "usage: demi host shell --host <name|id> <script>\n".into()
            )
        );
        assert_eq!(
            answers[4],
            (
                1,
                String::new(),
                "host shell: cross-host execution requires a machine job\n".into()
            )
        );
        pool.close().await;
    }
}

mod shells {
    use std::cell::Cell;

    use demi_shell::{Host as _, HostErrorKind};
    use demi_web_api::ids::{ConversationId, UserId};
    use futures_util::future::LocalBoxFuture;
    use tokio_util::sync::CancellationToken;

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use demi_backend_storage::accounts::TokenHash;
    use demi_backend_storage::control::testing;
    use demi_backend_storage::conversation_index::Creation;
    use demi_runner_protocol::wire::RunnerPlatform;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_job_runs_inside_the_host_access_and_is_refused_once_the_host_changed() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let laptop = control
            .create_device(
                owner.clone(),
                "laptop".into(),
                RunnerPlatform::Linux,
                TokenHash::of("laptop"),
            )
            .await
            .unwrap()
            .id;
        testing::execute(
            &control,
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?2, target_path = '/work'
             WHERE id = ?1",
            vec![ID.to_owned(), laptop.to_string()],
        )
        .await;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let _connection = shard.connect_for_tests(&laptop, "/home/ana");
                let host = shard.host_shard().conversation_host(&id).await.unwrap();
                assert_eq!(host.default_cwd(), "/work");
                let ran = Cell::new(0);
                let job = || {
                    let ran = &ran;
                    Box::pin(async move { ran.set(ran.get() + 1) }) as LocalBoxFuture<'_, ()>
                };
                let first = shard
                    .host_shard()
                    .run_job(&id, &host.key(), &CancellationToken::new(), job())
                    .await;
                // The conversation moves to another directory of the device:
                // its old Host runs nothing more.
                testing::execute(
                    &shard.services().control,
                    "UPDATE conversations SET target_path = '/elsewhere' WHERE id = ?1",
                    vec![ID.to_owned()],
                )
                .await;
                let second = shard
                    .host_shard()
                    .run_job(&id, &host.key(), &CancellationToken::new(), job())
                    .await;
                (first, second.map_err(|error| error.kind), ran.get())
            })
            .await
            .unwrap();
        assert_eq!(answers, (Ok(()), Err(HostErrorKind::Unavailable), 1));
        pool.close().await;
    }
}

mod remote_files {
    use demi_backend_host_access::remote_files::{RemoteFile, RemoteFileRefusal};
    use demi_core::UserContentBlock;
    use demi_web_api::ids::{ConversationId, DeviceId, UserId};

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use demi_backend_storage::accounts::TokenHash;
    use demi_backend_storage::control::testing;
    use demi_backend_storage::conversation_index::Creation;
    use demi_runner_protocol::wire::RunnerPlatform;

    #[tokio::test(flavor = "local")]
    async fn a_reference_keeps_its_device_and_path_and_an_inaccessible_one_grants_nothing() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        testing::execute(
            &control,
            "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
             VALUES ('other', 'other@example.test', '', 'unused', 'user', 0)",
            Vec::new(),
        )
        .await;
        let other = UserId::try_from("other").unwrap();
        let conversation =
            ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), conversation.clone())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let device = |user: UserId, token: &'static str| {
            let control = control.clone();
            async move {
                control
                    .create_device(
                        user,
                        "build".into(),
                        RunnerPlatform::Linux,
                        TokenHash::of(token),
                    )
                    .await
                    .unwrap()
                    .id
            }
        };
        let build = device(owner.clone(), "own").await;
        let foreign = device(other, "foreign").await;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        let referenced = build.clone();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let build = referenced;
                let path = "/srv/it's $(literal).txt".to_owned();
                let file = |device: &DeviceId| RemoteFile {
                    device: device.to_string(),
                    path: path.clone(),
                };
                // Offline, the device grants nothing.
                let offline = shard
                    .host_shard()
                    .reference_remote_files(&conversation, &[file(&build)])
                    .await;
                assert!(
                    matches!(offline, Err(RemoteFileRefusal::Offline(_))),
                    "{offline:?}"
                );
                let _connection = shard.connect_for_tests(&build, "/home/build");
                // One inaccessible reference refuses them all.
                let refused = shard
                    .host_shard()
                    .reference_remote_files(&conversation, &[file(&build), file(&foreign)])
                    .await;
                assert!(
                    matches!(refused, Err(RemoteFileRefusal::NotAccessible)),
                    "{refused:?}"
                );
                let attached_before = shard
                    .services()
                    .control
                    .attached_hosts(conversation.clone())
                    .await
                    .unwrap();
                let granted = shard
                    .host_shard()
                    .reference_remote_files(&conversation, &[file(&build)])
                    .await
                    .unwrap();
                let attached = shard
                    .services()
                    .control
                    .attached_hosts(conversation.clone())
                    .await
                    .unwrap();
                let record = shard
                    .host_shard()
                    .owned_conversation(&conversation)
                    .await
                    .unwrap();
                (
                    attached_before,
                    granted,
                    attached,
                    record.context_version,
                    path,
                )
            })
            .await
            .unwrap();
        let (attached_before, granted, attached, context_version, path) = answers;
        assert!(attached_before.is_empty());
        let [UserContentBlock::Reference { reference }] = granted.as_slice() else {
            panic!("expected one reference, got {granted:?}");
        };
        let url = url::Url::parse(reference).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["host"], "build");
        assert_eq!(query["deviceId"], build.as_str());
        assert_eq!(
            percent_encoding::percent_decode_str(url.path())
                .decode_utf8()
                .unwrap(),
            path
        );
        // The command reads the exact path, however the shell would read it.
        let command = shlex::split(&query["readCommand"]).unwrap();
        assert_eq!(
            command[..5],
            ["demi", "host", "shell", "--host", build.as_str()]
        );
        assert_eq!(
            shlex::split(&command[5]).unwrap(),
            ["cat", "--", path.as_str()]
        );
        // The device is attached once, which the nodes hear of.
        assert_eq!(
            attached.iter().map(|host| &host.device).collect::<Vec<_>>(),
            [&build]
        );
        assert_eq!(context_version, 1);
        pool.close().await;
    }
}
