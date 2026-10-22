//! Conservative native JSONL replay: equal sequence numbers are not identities.
//! Identical repeats and forward gaps remain readable; conflicting work and
//! broken ancestry must be reported without rewriting the original bytes.

use std::sync::Arc;

use ice_agent::message::AgentMessage;
use ice_harness::session::jsonl::{encode_mutation, JsonlSessionStorage, JsonlV4Header};
use ice_harness::session::memory::{CounterIdGenerator, FakeClock};
use ice_harness::session::types::{
    provisioned_into_entry, ProvisionedEntry, ProvisionedKind, SessionMutation, SessionStorage,
};
use ice_tools::env::{FileContent, FileSystem};

fn user_msg(text: &str) -> AgentMessage {
    AgentMessage::User(ice_ai::types::UserMessage::new(text, 1))
}

fn header() -> JsonlV4Header {
    JsonlV4Header::new(
        "sess-1".into(),
        1_700_000_000_000,
        "/cwd".into(),
        None,
        None,
        None,
    )
}

type Fixture = (Arc<dyn FileSystem>, Arc<FakeClock>, Arc<CounterIdGenerator>);

fn fixture() -> Fixture {
    let env = ice_tools::InMemoryExecutionEnv::with_cwd("/".into());
    (
        Arc::new(env),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    )
}

fn message_provisioned(id: &str, text: &str) -> ProvisionedEntry {
    ProvisionedEntry {
        id: id.to_string(),
        kind: ProvisionedKind::Message {
            message: user_msg(text),
            terminate: None,
        },
    }
}

fn entry_mutation(seq: u64, id: &str, parent: Option<&str>) -> SessionMutation {
    let timestamp = 1_700_000_000_000i64 + seq as i64;
    SessionMutation::Entry {
        seq,
        timestamp,
        lane: Some("main".into()),
        entry: provisioned_into_entry(
            message_provisioned(id, id),
            seq,
            parent.map(str::to_string),
            timestamp,
        ),
    }
}

fn lane_mutation(seq: u64) -> SessionMutation {
    SessionMutation::Lane {
        seq,
        lane: "main".into(),
        leaf_id: None,
    }
}

async fn seed(fs: &Arc<dyn FileSystem>, mutations: &[SessionMutation]) {
    JsonlSessionStorage::create(
        fs.clone(),
        "/s.jsonl",
        header(),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    )
    .await
    .unwrap();
    for mutation in mutations {
        fs.append_file(
            "/s.jsonl",
            FileContent::Text(encode_mutation(mutation)),
            None,
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn identical_repeat_and_valid_forward_gap_preserve_bytes_and_append_sequence() {
    let (fs, clock, ids) = fixture();
    seed(
        &fs,
        &[
            lane_mutation(1),
            entry_mutation(2, "e1", None),
            entry_mutation(3, "e2", Some("e1")),
            entry_mutation(3, "e2", Some("e1")),
            entry_mutation(10, "e3", Some("e2")),
        ],
    )
    .await;
    let before = fs.read_text_file("/s.jsonl", None).await.unwrap();
    let storage = JsonlSessionStorage::load(fs.clone(), "/s.jsonl", clock, ids)
        .await
        .unwrap();
    assert_eq!(fs.read_text_file("/s.jsonl", None).await.unwrap(), before);
    assert_eq!(
        storage.get_entry("e3").await.unwrap().unwrap().parent_id(),
        Some("e2")
    );
    let next = storage
        .append_entry(message_provisioned("e4", "new work"), "main")
        .await
        .unwrap();
    assert_eq!(next.seq(), 11);
}

async fn assert_rejected_without_rewrite(mutations: &[SessionMutation]) {
    let (fs, clock, ids) = fixture();
    seed(&fs, mutations).await;
    let before = fs.read_text_file("/s.jsonl", None).await.unwrap();
    let result = JsonlSessionStorage::load(fs.clone(), "/s.jsonl", clock, ids).await;
    assert!(
        result.is_err(),
        "conflicting history must not be silently healed"
    );
    assert_eq!(fs.read_text_file("/s.jsonl", None).await.unwrap(), before);
}

#[tokio::test]
async fn equal_sequence_with_different_identity_is_rejected_without_rewrite() {
    assert_rejected_without_rewrite(&[
        lane_mutation(1),
        entry_mutation(2, "e1", None),
        entry_mutation(2, "other-work", None),
    ])
    .await;
}

#[tokio::test]
async fn equal_sequence_with_different_payload_is_rejected_without_rewrite() {
    let timestamp = 1_700_000_000_002;
    let changed = SessionMutation::Entry {
        seq: 2,
        timestamp,
        lane: Some("main".into()),
        entry: provisioned_into_entry(
            message_provisioned("e1", "different work"),
            2,
            None,
            timestamp,
        ),
    };
    assert_rejected_without_rewrite(&[lane_mutation(1), entry_mutation(2, "e1", None), changed])
        .await;
}

#[tokio::test]
async fn equal_sequence_with_different_lane_is_rejected_without_rewrite() {
    let mut changed = entry_mutation(2, "e1", None);
    if let SessionMutation::Entry { lane, .. } = &mut changed {
        *lane = Some("other".into());
    }
    assert_rejected_without_rewrite(&[lane_mutation(1), entry_mutation(2, "e1", None), changed])
        .await;
}

#[tokio::test]
async fn equal_sequence_with_different_parent_is_rejected_without_rewrite() {
    assert_rejected_without_rewrite(&[
        lane_mutation(1),
        entry_mutation(2, "e1", None),
        entry_mutation(3, "e2", Some("e1")),
        entry_mutation(3, "e2", None),
    ])
    .await;
}

#[tokio::test]
async fn stale_parent_is_rejected_instead_of_silently_reparented() {
    assert_rejected_without_rewrite(&[
        lane_mutation(1),
        entry_mutation(2, "e1", None),
        entry_mutation(3, "e2", None),
    ])
    .await;
}

#[tokio::test]
async fn conflict_before_torn_tail_does_not_publish_a_repair() {
    let (fs, clock, ids) = fixture();
    seed(
        &fs,
        &[
            lane_mutation(1),
            entry_mutation(2, "e1", None),
            entry_mutation(2, "different", None),
        ],
    )
    .await;
    fs.append_file("/s.jsonl", FileContent::Text("{torn".into()), None)
        .await
        .unwrap();
    let before = fs.read_text_file("/s.jsonl", None).await.unwrap();
    assert!(
        JsonlSessionStorage::load(fs.clone(), "/s.jsonl", clock, ids)
            .await
            .is_err()
    );
    assert_eq!(fs.read_text_file("/s.jsonl", None).await.unwrap(), before);
}
