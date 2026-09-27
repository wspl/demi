//! The tree store contract over the in-memory store (`subagents.md`
//! § Persistence); the cases are `agent::testing::store_contract`'s, which
//! the backend's database store passes too.

use demi_agent::testing::{MemoryTreeStore, store_contract};

#[tokio::test(flavor = "local")]
async fn create_queues_the_first_message_with_the_node_and_a_save_replaces_it() {
    store_contract::create_queues_the_first_message_with_the_node_and_a_save_replaces_it(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_save_delivers_the_completions_its_transcript_carries_and_only_those() {
    store_contract::a_save_delivers_the_completions_its_transcript_carries_and_only_those(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree() {
    store_contract::reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_completion_of_an_earlier_round_marks_the_current_one_undelivered() {
    store_contract::a_completion_of_an_earlier_round_marks_the_current_one_undelivered(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn the_blob_namespace_names_bytes_by_their_sha256() {
    store_contract::the_blob_namespace_names_bytes_by_their_sha256(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_save_delivers_a_completion_it_holds_as_waiting_input() {
    store_contract::a_save_delivers_a_completion_it_holds_as_waiting_input(&*MemoryTreeStore::new())
        .await;
}

#[tokio::test(flavor = "local")]
async fn children_list_in_spawn_order_and_a_close_keeps_its_result() {
    store_contract::children_list_in_spawn_order_and_a_close_keeps_its_result(
        &*MemoryTreeStore::new(),
    )
    .await;
}
