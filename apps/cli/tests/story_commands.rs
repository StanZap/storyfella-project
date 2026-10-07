use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!(
                "story-cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.dir.join(name), text).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_svs"))
            .args(args)
            .current_dir(&self.dir)
            .env("SVS_HOME", self.dir.join("missing-config-root"))
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str], success: bool) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.success(), success, "{:?}", output);
        assert!(
            !self.dir.join(".svs-project.db").exists(),
            "story commands must not open a DB"
        );
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {:?}", output))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn neutral_story_check_and_compile_work_without_config_or_database() {
    let fixture = Fixture::new();
    fixture.write(
        "story.sf",
        "scene first\n  narrate: Hello\nscene next\n  narrate: Goodbye\n",
    );
    let report = fixture.json(&["story", "check", "story.sf", "--require-ready"], true);
    assert_eq!(report["valid"], true);
    assert_eq!(report["ready"], true);
    let plan = fixture.json(&["story", "compile", "story.sf"], true);
    assert_eq!(plan["version"], 1);
    assert_eq!(plan["entry"], "first");
    assert_eq!(
        plan["scenes"][0]["instructions"][1]["kind"]["Jump"]["target"],
        "next"
    );
    assert_eq!(
        plan["scenes"][0]["instructions"][1]["origin"],
        "SceneBoundary"
    );
}

#[test]
fn draft_plan_preserves_missing_assets_and_directions_but_readiness_gate_fails() {
    let fixture = Fixture::new();
    let text = "scene harbor\n  shot: wide\n  mara: Hello\n";
    fixture.write("story.sf", text);
    let report = fixture.json(&["story", "check", "story.sf"], true);
    assert_eq!(report["valid"], true);
    assert_eq!(report["ready"], false);
    let plan = fixture.json(&["story", "compile", "story.sf"], true);
    assert_eq!(plan["source"], text);
    assert_eq!(
        plan["readiness"]["pending_assets"][0]["name"]["value"],
        "mara"
    );
    assert_eq!(
        plan["readiness"]["unsupported_directions"][0]["text"]["value"],
        "wide"
    );
    let blocked = fixture.json(&["story", "compile", "story.sf", "--require-ready"], false);
    assert_eq!(blocked["valid"], true);
    assert_eq!(blocked["ready"], false);
}

#[test]
fn binding_file_is_snapshotted_exactly_and_does_not_create_readiness_for_directions() {
    let fixture = Fixture::new();
    fixture.write("story.sf", "scene harbor\n  Mara: Hello\n");
    let binding = json!({"id": "character-1", "name": "mara", "kind": "Character", "revision": "revision-sha256-1"});
    fixture.write("assets.json", &json!([binding.clone()]).to_string());
    let plan = fixture.json(
        &[
            "story",
            "compile",
            "story.sf",
            "--assets",
            "assets.json",
            "--require-ready",
        ],
        true,
    );
    assert_eq!(
        plan["scenes"][0]["instructions"][0]["kind"]["Speech"]["speaker"]["binding"],
        binding
    );
    fixture.write("story.sf", "scene harbor\n  mood: tense\n  Mara: Hello\n");
    let blocked = fixture.json(
        &[
            "story",
            "check",
            "story.sf",
            "--assets",
            "assets.json",
            "--require-ready",
        ],
        false,
    );
    assert_eq!(blocked["readiness"]["pending_assets"], json!([]));
    assert_eq!(
        blocked["readiness"]["unsupported_directions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn semantic_failures_emit_positioned_diagnostics_without_touching_source() {
    let fixture = Fixture::new();
    for text in [
        "scene first\n  goto absent\n",
        "scene first\n  end\n  narrate: unreachable\n",
    ] {
        fixture.write("story.sf", text);
        let report = fixture.json(&["story", "compile", "story.sf"], false);
        assert_eq!(report["valid"], false);
        assert!(report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["severity"] == "Error" && d["start"]["line"].is_number()));
        assert_eq!(
            std::fs::read_to_string(fixture.dir.join("story.sf")).unwrap(),
            text
        );
    }
}

#[test]
fn malformed_binding_schema_is_rejected() {
    let fixture = Fixture::new();
    fixture.write("story.sf", "scene first\n  narrate: Hello\n");
    fixture.write(
        "assets.json",
        r#"[{"id":"a","name":"mara","kind":"Character","revision":"r","legacy_version":1}]"#,
    );
    let output = fixture.run(&["story", "compile", "story.sf", "--assets", "assets.json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid asset binding array"));
}

#[test]
fn catalog_entries_without_images_are_resolved_but_report_pending_artwork() {
    let fixture = Fixture::new();
    fixture.write("story.sf", "scene first\n  prop: lantern\n");
    fixture.write(
        "assets.json",
        r#"[{"id":"lantern-id","name":"lantern","kind":"Prop"}]"#,
    );
    let report = fixture.json(
        &["story", "check", "story.sf", "--assets", "assets.json"],
        true,
    );
    assert_eq!(report["valid"], true);
    assert_eq!(report["ready"], false);
    assert!(report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "SF306"));
    assert!(!report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "SF203"));
}
