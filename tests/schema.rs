//! `--emit-ast-schema` is a published interface — LLM structured-output
//! modes are configured against the dialect it emits, so a change of
//! dialect breaks downstream callers even when the shape is unchanged.
//! `schemars` picks the dialect, and the 0.9 → 1.0 bump moved it from
//! draft-07 to 2020-12 with nothing to catch it. These tests pin it.

use std::process::Command;

fn emit_ast_schema() -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_adoc"))
        .arg("--emit-ast-schema")
        .output()
        .expect("failed to run adoc --emit-ast-schema");
    assert!(
        out.status.success(),
        "adoc --emit-ast-schema exited with {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("--emit-ast-schema did not emit valid JSON")
}

#[test]
fn schema_declares_the_2020_12_dialect() {
    let schema = emit_ast_schema();
    assert_eq!(
        schema["$schema"].as_str(),
        Some("https://json-schema.org/draft/2020-12/schema"),
        "the emitted JSON Schema dialect changed; this is a breaking change \
         to a published interface — update the docs deliberately rather than \
         relaxing this test"
    );
}

#[test]
fn schema_describes_the_document_type() {
    let schema = emit_ast_schema();
    assert_eq!(schema["title"].as_str(), Some("Document"));
    assert_eq!(schema["type"].as_str(), Some("object"));
    for key in ["attributes", "blocks", "header"] {
        assert!(
            schema["properties"].get(key).is_some(),
            "schema is missing the `{key}` property"
        );
    }
}
