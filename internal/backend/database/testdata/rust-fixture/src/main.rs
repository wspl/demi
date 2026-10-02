use demi_backend_database::{
    control::{ControlService, testing},
    conversations::ConversationStores,
    sequences,
};
use demi_shared_types::{Sequence, SystemClock};
use demi_web_api_protocol::ids::ConversationId;
use std::{path::Path, sync::Arc};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let directory = std::env::args().nth(1).expect("fixture directory");
    let directory = Path::new(&directory);
    let control = ControlService::open(
        &directory.join("rust-control.sqlite"),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let user = testing::master(&control).await;
    let id = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
    control
        .create_conversation(user.id, id.clone())
        .await
        .unwrap();
    control.close().await.unwrap();
    let stores = ConversationStores::open(
        directory.join("rust-conversations"),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .await
    .unwrap();
    stores
        .db(&id)
        .call(|connection| sequences::next(connection, Sequence::Command))
        .await
        .unwrap();
    assert!(stores.close().await.is_empty());
}
