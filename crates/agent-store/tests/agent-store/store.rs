//! The tree store contract over the in-memory store
//! (`crates/agent-store/src/lib.rs`).
//!
//! The cases are `agent_store::testing::store_contract`'s, which the backend's
//! database store passes too.

use demi_agent_store::testing::{MemoryTreeStore, store_contract};

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
async fn reopen_makes_a_closed_node_live_with_its_message() {
    store_contract::reopen_makes_a_closed_node_live_with_its_message(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_reopen_with_a_profile_replaces_what_the_profile_chose() {
    store_contract::a_reopen_with_a_profile_replaces_what_the_profile_chose(
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
async fn children_list_in_spawn_order_and_a_close_keeps_its_phase() {
    store_contract::children_list_in_spawn_order_and_a_close_keeps_its_phase(
        &*MemoryTreeStore::new(),
    )
    .await;
}

#[tokio::test(flavor = "local")]
async fn each_sequence_gives_its_numbers_once_in_order() {
    store_contract::each_sequence_gives_its_numbers_once_in_order(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_subagent_tells_whether_its_start_is_on_its_parents_path() {
    store_contract::a_subagent_tells_whether_its_start_is_on_its_parents_path(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_compaction_keeps_the_versions_of_what_it_carries() {
    store_contract::a_compaction_keeps_the_versions_of_what_it_carries(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_rewrite_keeps_its_rows_and_a_switch_brings_them_back() {
    store_contract::a_rewrite_keeps_its_rows_and_a_switch_brings_them_back(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_side_question_is_no_subagent_and_keeps_what_it_asks() {
    store_contract::a_side_question_is_no_subagent_and_keeps_what_it_asks(&*MemoryTreeStore::new()).await;
}

#[tokio::test(flavor = "local")]
async fn a_load_reads_the_window_and_the_rest_on_demand() {
    store_contract::a_load_reads_the_window_and_the_rest_on_demand(&*MemoryTreeStore::new()).await;
}
