use std::fs;

use dotall_core::{CachedDerived, DerivationRecipe, DotallStore};
use tempfile::tempdir;

fn fixture() -> (tempfile::TempDir, DotallStore) {
    let temp = tempdir().expect("tempdir");
    fs::write(temp.path().join("book.xlsx"), b"source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register source");
    (temp, store)
}

fn recipe(store: &DotallStore) -> DerivationRecipe {
    DerivationRecipe {
        source_hash: store.manifest().objects["book.xlsx"]
            .fingerprint
            .blake3
            .clone(),
        processor_id: "xlsx.formula-dependencies".into(),
        processor_version: "1".into(),
        config_hash: "formula-parser-v1".into(),
        input_hashes: vec!["model-v1".into()],
    }
}

fn derived(recipe: &DerivationRecipe) -> CachedDerived {
    CachedDerived {
        source_hash: "stale-caller-hash".into(),
        model_schema_id: "xlsx.workbook".into(),
        model_schema_version: 1,
        processor_id: recipe.processor_id.clone(),
        processor_version: recipe.processor_version.clone(),
        payload: serde_json::json!({"dependencies": []}),
    }
}

#[test]
fn recipe_keyed_derived_cache_hits_only_for_matching_recipe_identity() {
    let (temp, store) = fixture();
    let recipe = recipe(&store);
    let expected = CachedDerived {
        source_hash: recipe.source_hash.clone(),
        ..derived(&recipe)
    };

    store
        .write_derived_for_recipe("book.xlsx", &recipe, &derived(&recipe))
        .expect("write derived");

    assert_eq!(
        store
            .read_derived_for_recipe("book.xlsx", &recipe, "xlsx.workbook", 1)
            .expect("read derived"),
        Some(expected)
    );

    let mut different_config = recipe.clone();
    different_config.config_hash = "formula-parser-v2".into();
    assert_eq!(
        store
            .read_derived_for_recipe("book.xlsx", &different_config, "xlsx.workbook", 1)
            .expect("read different config"),
        None
    );

    let cache_path = temp.path().join(format!(
        ".all/objects/book.xlsx/cache/derived/{}.json",
        recipe.key().expect("recipe key")
    ));
    let mut cached: serde_json::Value =
        serde_json::from_slice(&fs::read(&cache_path).expect("read cache")).expect("decode cache");
    cached["source_hash"] = serde_json::json!("wrong-source-hash");
    fs::write(
        &cache_path,
        serde_json::to_vec(&cached).expect("encode source mismatch"),
    )
    .expect("write source mismatch");
    assert_eq!(
        store
            .read_derived_for_recipe("book.xlsx", &recipe, "xlsx.workbook", 1)
            .expect("read source mismatch"),
        None
    );

    store
        .write_derived_for_recipe("book.xlsx", &recipe, &derived(&recipe))
        .expect("rewrite derived");
    let mut cached: serde_json::Value =
        serde_json::from_slice(&fs::read(&cache_path).expect("read cache")).expect("decode cache");
    cached["processor_version"] = serde_json::json!("2");
    fs::write(
        &cache_path,
        serde_json::to_vec(&cached).expect("encode processor mismatch"),
    )
    .expect("write processor mismatch");
    assert_eq!(
        store
            .read_derived_for_recipe("book.xlsx", &recipe, "xlsx.workbook", 1)
            .expect("read processor mismatch"),
        None
    );
}
