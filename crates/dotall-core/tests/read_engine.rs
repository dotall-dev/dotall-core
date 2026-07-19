use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dotall_core::read::apply_budget;
use dotall_core::registry::{
    ArtifactSchema, Capability, DetectionProbe, DetectionScore, FormatDescriptor, FormatHandler,
    FormatRegistry, Inspection, ReadRequest,
};
use dotall_core::{ArtifactEnvelope, DotallStore, Engine, ReadResponse, Result};
use tempfile::tempdir;

#[test]
fn budget_truncation_provides_a_lossless_continuation() {
    let content = "row one\nrow two\nrow three\n";

    let (first, truncated, cursor) = apply_budget(content, 3, 0);
    let (second, second_truncated, second_cursor) = apply_budget(
        content,
        20,
        cursor
            .as_deref()
            .expect("continuation")
            .parse()
            .expect("numeric cursor"),
    );

    assert!(truncated);
    assert!(!second_truncated);
    assert_eq!(second_cursor, None);
    assert_eq!(format!("{first}{second}"), content);
}

#[test]
fn budget_cursor_remains_valid_for_unicode_content() {
    let content = "ééééé";

    let (first, truncated, cursor) = apply_budget(content, 1, 0);
    let (second, _, _) = apply_budget(
        content,
        20,
        cursor
            .as_deref()
            .expect("continuation")
            .parse()
            .expect("numeric cursor"),
    );

    assert!(truncated);
    assert_eq!(first.chars().count(), 4);
    assert_eq!(format!("{first}{second}"), content);
}

#[test]
fn second_inspection_uses_cached_model_without_reparse() {
    let mut fixture = EngineFixture::new();

    fixture.engine.inspect("sample.stub").expect("cold inspect");
    fixture.engine.inspect("sample.stub").expect("warm inspect");

    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 1);
}

#[test]
fn inspection_normalizes_dot_relative_path_and_reuses_cached_model() {
    let mut fixture = EngineFixture::new();

    let first = fixture
        .engine
        .inspect("./sample.stub")
        .expect("cold inspect with dot-relative path");
    let second = fixture
        .engine
        .inspect("./sample.stub")
        .expect("warm inspect with dot-relative path");

    assert!(!first.model_cache_hit);
    assert!(second.model_cache_hit);
    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 1);
}

#[test]
fn stale_source_reparses_before_inspection() {
    let mut fixture = EngineFixture::new();

    fixture.engine.inspect("sample.stub").expect("cold inspect");
    fs::write(fixture.root.join("sample.stub"), b"changed").expect("overwrite source");
    fixture
        .engine
        .inspect("sample.stub")
        .expect("stale inspect");

    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 2);
}

#[test]
fn mismatched_cached_producer_reparses_before_inspection() {
    let mut fixture = EngineFixture::new();

    fixture.engine.inspect("sample.stub").expect("cold inspect");
    let model_path = fixture
        .root
        .join(".all/objects/sample.stub/cache/model/model.json");
    let mut cached: serde_json::Value =
        serde_json::from_slice(&fs::read(&model_path).expect("read cached model"))
            .expect("decode cached model");
    cached["producer_version"] = serde_json::json!("outdated");
    fs::write(
        &model_path,
        serde_json::to_vec(&cached).expect("encode cached model"),
    )
    .expect("overwrite cached model");

    let result = fixture
        .engine
        .inspect("sample.stub")
        .expect("reparse mismatched cache");

    assert!(!result.model_cache_hit);
    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 2);
}

#[test]
fn read_applies_budget_and_reuses_rendered_view() {
    let mut fixture = EngineFixture::new();
    let request = ReadRequest {
        selector: None,
        max_tokens: 3,
        continuation: None,
    };

    let first = fixture
        .engine
        .read("sample.stub", &request)
        .expect("first read");
    let second = fixture
        .engine
        .read("sample.stub", &request)
        .expect("cached read");

    assert!(first.response.truncated);
    assert!(first.response.continuation.is_some());
    assert!(!first.view_cache_hit);
    assert!(second.view_cache_hit);
    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 1);
}

#[test]
fn inspection_and_read_append_access_records() {
    let mut fixture = EngineFixture::new();
    let request = ReadRequest {
        selector: None,
        max_tokens: 100,
        continuation: None,
    };

    fixture.engine.inspect("sample.stub").expect("inspect");
    fixture.engine.read("sample.stub", &request).expect("read");

    let log = fs::read_to_string(
        fixture
            .root
            .join(".all/objects/sample.stub/state/access/log.jsonl"),
    )
    .expect("access log");
    let records: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON record"))
        .collect();

    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["operation"], "inspect");
    assert_eq!(records[1]["operation"], "read");
    assert_eq!(records[1]["path"], "sample.stub");
}

struct EngineFixture {
    _workspace: tempfile::TempDir,
    root: std::path::PathBuf,
    engine: Engine,
    parse_count: Arc<AtomicUsize>,
}

impl EngineFixture {
    fn new() -> Self {
        let temp = tempdir().expect("workspace");
        let root = temp.path().to_path_buf();
        fs::write(root.join("sample.stub"), b"initial").expect("source");
        let store = DotallStore::init(&root).expect("store");
        let parse_count = Arc::new(AtomicUsize::new(0));
        let mut registry = FormatRegistry::default();
        registry.register(Arc::new(CountingFormat {
            parse_count: Arc::clone(&parse_count),
        }));

        Self {
            _workspace: temp,
            root,
            engine: Engine::new(store, registry),
            parse_count,
        }
    }
}

struct CountingFormat {
    parse_count: Arc<AtomicUsize>,
}

impl FormatHandler for CountingFormat {
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
        DetectionScore(
            probe
                .path
                .extension()
                .is_some_and(|extension| extension == "stub") as u16,
        )
    }

    fn parse(&self, _source: &Path) -> Result<ArtifactEnvelope> {
        self.parse_count.fetch_add(1, Ordering::SeqCst);
        Ok(ArtifactEnvelope {
            format_id: "stub".into(),
            schema_id: "stub.document".into(),
            schema_version: 1,
            payload: serde_json::json!({}),
        })
    }

    fn inspect(&self, _model: &ArtifactEnvelope) -> Result<Inspection> {
        Ok(Inspection {
            format_id: "stub".into(),
            summary: serde_json::json!({"title": "Stub"}),
            capabilities: self.descriptor().capabilities,
            suggested_reads: Vec::new(),
        })
    }

    fn read(&self, _model: &ArtifactEnvelope, _request: &ReadRequest) -> Result<ReadResponse> {
        Ok(ReadResponse {
            content: "row one\nrow two\nrow three\n".into(),
            estimated_tokens: 7,
            truncated: false,
            continuation: None,
            next_actions: Vec::new(),
        })
    }
}
