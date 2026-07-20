use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dotall_core::registry::{
    ArtifactSchema, Capability, DetectionProbe, DetectionScore, FormatDescriptor, FormatHandler,
    FormatRegistry, Inspection, PatchedOutput, ReadRequest,
};
use dotall_core::{
    Actor, ActorKind, ArtifactEnvelope, DependencyImpact, DotallError, DotallStore, EditRequest,
    Engine, ReadResponse, Result, SemanticChange, SemanticOperation, ValidatedEdit,
};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn edit_stages_without_mutating_source() {
    let mut fixture = Fixture::new();
    let before = fs::read(fixture.source()).expect("source before");
    let hash = fixture.source_hash();

    let staged = fixture
        .engine
        .edit("sample.stub", &request(hash, "updated", tx(1)))
        .expect("stage edit");

    assert_eq!(fs::read(fixture.source()).expect("source after"), before);
    assert_eq!(staged.tx_id, tx(1));
    assert!(
        fixture
            .root
            .join(format!(
                ".all/objects/sample.stub/state/edits/staging/{}.json",
                tx(1)
            ))
            .is_file()
    );
}

#[test]
fn apply_updates_source_history_and_model() {
    let mut fixture = Fixture::new();
    let hash = fixture.source_hash();
    fixture
        .engine
        .edit("sample.stub", &request(hash, "updated", tx(1)))
        .expect("stage");

    let applied = fixture
        .engine
        .apply("sample.stub", tx(1))
        .expect("apply staged edit");

    assert_eq!(applied.version, 1);
    assert_eq!(fs::read(fixture.source()).expect("source"), b"updated");
    assert_eq!(
        fixture
            .engine
            .history("sample.stub")
            .expect("history")
            .len(),
        1
    );
    assert!(
        fixture
            .engine
            .load_model("sample.stub")
            .expect("refreshed model")
            .envelope
            .payload["content"]
            == "updated"
    );
    assert!(fixture.parse_count.load(Ordering::SeqCst) >= 2);
}

#[test]
fn apply_rejects_a_stale_expected_source_hash() {
    let mut fixture = Fixture::new();
    let hash = fixture.source_hash();
    fixture
        .engine
        .edit("sample.stub", &request(hash, "updated", tx(1)))
        .expect("stage");
    fs::write(fixture.source(), b"external change").expect("external write");

    let error = fixture
        .engine
        .apply("sample.stub", tx(1))
        .expect_err("stale source must reject apply");

    assert!(matches!(error, DotallError::SourceHashMismatch { .. }));
    assert_eq!(
        fs::read(fixture.source()).expect("source"),
        b"external change"
    );
    assert!(
        fixture
            .engine
            .history("sample.stub")
            .expect("history")
            .is_empty()
    );
}

#[test]
fn duplicate_transaction_id_returns_the_existing_stage_and_apply_result() {
    let mut fixture = Fixture::new();
    let hash = fixture.source_hash();
    let request = request(hash, "updated", tx(1));

    let first = fixture.engine.edit("sample.stub", &request).expect("stage");
    let retry = fixture
        .engine
        .edit("sample.stub", &request)
        .expect("retry stage");
    assert_eq!(first, retry);

    let applied = fixture.engine.apply("sample.stub", tx(1)).expect("apply");
    let retry = fixture
        .engine
        .apply("sample.stub", tx(1))
        .expect("retry apply");
    assert_eq!(applied, retry);
    assert_eq!(
        fixture
            .engine
            .history("sample.stub")
            .expect("history")
            .len(),
        1
    );
}

#[test]
fn revert_stages_snapshot_restore_and_commits_a_new_version() {
    let mut fixture = Fixture::new();
    let initial_hash = fixture.source_hash();
    fixture
        .engine
        .edit("sample.stub", &request(initial_hash, "updated", tx(1)))
        .expect("stage original");
    fixture
        .engine
        .apply("sample.stub", tx(1))
        .expect("apply original");

    let staged = fixture
        .engine
        .revert("sample.stub", 1, tx(2))
        .expect("stage revert");
    assert_eq!(
        fs::read(fixture.source()).expect("still updated"),
        b"updated"
    );
    assert_eq!(staged.preview.operations[0].kind, "restore_snapshot");

    let reverted = fixture
        .engine
        .apply("sample.stub", tx(2))
        .expect("apply revert");
    assert_eq!(reverted.version, 2);
    assert_eq!(fs::read(fixture.source()).expect("restored"), b"initial");
    assert_eq!(
        fixture
            .engine
            .diff("sample.stub", 2)
            .expect("revert record")
            .revert_of,
        Some(1)
    );
}

#[test]
fn recover_completes_a_committed_journal_after_interruption() {
    let mut fixture = Fixture::new();
    let hash = fixture.source_hash();
    fixture
        .engine
        .edit("sample.stub", &request(hash, "updated", tx(1)))
        .expect("stage");
    fixture
        .engine
        .simulate_interruption_after_commit("sample.stub", tx(1))
        .expect("write durable commit state");

    fixture.engine.recover("sample.stub").expect("recover");

    assert_eq!(fs::read(fixture.source()).expect("source"), b"updated");
    assert_eq!(
        fixture
            .engine
            .history("sample.stub")
            .expect("history")
            .len(),
        1
    );
    assert!(
        fixture
            .engine
            .recover("sample.stub")
            .expect("idempotent recovery")
            == 0
    );
}

#[test]
fn recover_completes_a_replace_durable_before_commit_marker() {
    let mut fixture = Fixture::new();
    let hash = fixture.source_hash();
    fixture
        .engine
        .edit("sample.stub", &request(hash, "updated", tx(1)))
        .expect("stage");
    fixture
        .engine
        .simulate_interruption_after_replace("sample.stub", tx(1))
        .expect("replace source before commit marker");

    assert_eq!(fs::read(fixture.source()).expect("source"), b"updated");
    assert_eq!(fixture.engine.recover("sample.stub").expect("recover"), 1);
    assert_eq!(
        fixture
            .engine
            .history("sample.stub")
            .expect("history")
            .len(),
        1
    );
    assert_eq!(
        fixture
            .engine
            .apply("sample.stub", tx(1))
            .expect("idempotent apply")
            .version,
        1
    );
    assert_eq!(
        fixture
            .engine
            .recover("sample.stub")
            .expect("idempotent recovery"),
        0
    );
}

struct Fixture {
    _workspace: tempfile::TempDir,
    root: std::path::PathBuf,
    engine: Engine,
    parse_count: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        let workspace = tempdir().expect("workspace");
        let root = workspace.path().to_path_buf();
        fs::write(root.join("sample.stub"), b"initial").expect("source");
        let store = DotallStore::init(&root).expect("store");
        let parse_count = Arc::new(AtomicUsize::new(0));
        let mut registry = FormatRegistry::default();
        registry.register(Arc::new(StubFormat {
            parse_count: Arc::clone(&parse_count),
        }));
        Self {
            _workspace: workspace,
            root,
            engine: Engine::new(store, registry),
            parse_count,
        }
    }

    fn source(&self) -> std::path::PathBuf {
        self.root.join("sample.stub")
    }

    fn source_hash(&mut self) -> String {
        self.engine
            .load_model("sample.stub")
            .expect("load model")
            .source_hash
    }
}

fn tx(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn request(expected_source_hash: String, value: &str, transaction_id: Uuid) -> EditRequest {
    EditRequest {
        transaction_id,
        expected_source_hash,
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("test".into()),
        },
        operations: vec![SemanticOperation {
            kind: "replace".into(),
            payload: serde_json::json!({ "content": value }),
        }],
    }
}

struct StubFormat {
    parse_count: Arc<AtomicUsize>,
}

impl FormatHandler for StubFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: "stub".into(),
            version: "1".into(),
            capabilities: vec![Capability::Inspect, Capability::ReadFull],
        }
    }

    fn artifact_schema(&self) -> ArtifactSchema {
        ArtifactSchema {
            format_id: "stub".into(),
            schema_id: "stub.document".into(),
            schema_version: 1,
        }
    }

    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore {
        DetectionScore((probe.path.extension() == Some("stub".as_ref())) as u16)
    }

    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope> {
        self.parse_count.fetch_add(1, Ordering::SeqCst);
        let content =
            String::from_utf8(fs::read(source).expect("test source")).expect("test source UTF-8");
        Ok(ArtifactEnvelope {
            format_id: "stub".into(),
            schema_id: "stub.document".into(),
            schema_version: 1,
            payload: serde_json::json!({ "content": content }),
        })
    }

    fn inspect(&self, _model: &ArtifactEnvelope) -> Result<Inspection> {
        unreachable!("transactions only")
    }

    fn read(&self, _model: &ArtifactEnvelope, _request: &ReadRequest) -> Result<ReadResponse> {
        unreachable!("transactions only")
    }

    fn validate_edit(
        &self,
        _model: &ArtifactEnvelope,
        operations: &[SemanticOperation],
    ) -> Result<ValidatedEdit> {
        let value = operations[0].payload["content"].as_str().expect("content");
        Ok(ValidatedEdit {
            format_id: "stub".into(),
            schema_id: "stub.edits".into(),
            schema_version: 1,
            operations: operations.to_vec(),
            semantic_diff: vec![SemanticChange {
                target: "content".into(),
                element_id: "content".into(),
                change: "replace".into(),
                before: None,
                after: Some(value.into()),
            }],
            dependency_impact: DependencyImpact {
                forward: Vec::new(),
                notes: Vec::new(),
            },
        })
    }

    fn apply_edit(&self, _source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
        let bytes = edit.operations[0].payload["content"]
            .as_str()
            .expect("content")
            .as_bytes()
            .to_vec();
        Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        })
    }
}
