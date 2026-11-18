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
        Command::new(env!("CARGO_BIN_EXE_sf-cli"))
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
fn cli_exposes_story_and_mcp_without_legacy_image_commands() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("story"));
    assert!(help.contains("mcp"));
    assert!(!help.contains("runtime"));
    assert!(!help.contains("stack"));
    assert!(!help.contains("eval"));
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
fn multi_file_story_compiles_and_reports_original_file_locations() {
    let fixture = Fixture::new();
    fixture.write("story.sf", "include \"first.sf\"\ninclude \"second.sf\"\n");
    fixture.write("first.sf", "scene beginning\n  goto ending\n");
    fixture.write("second.sf", "scene ending\n  narrate: 🌊\n  end\n");
    let plan = fixture.json(&["story", "compile", "story.sf"], true);
    assert_eq!(plan["entry"], "beginning");
    assert_eq!(plan["scenes"][1]["id"], "ending");
    fixture.write("plan.json", &plan.to_string());
    fixture.write("commands.json", "[\"Advance\"]");
    let trace = fixture.json(&["story", "trace", "plan.json", "commands.json"], true);
    assert!(trace
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event.get("Completed").is_some()));
    fixture.write("first.sf", "scene beginning\n  goto missing\n");
    let report = fixture.json(&["story", "check", "story.sf"], false);
    let diagnostic = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "SF202")
        .unwrap();
    assert_eq!(diagnostic["file"], "first.sf");
    assert_eq!(diagnostic["start"]["line"], 1);
    assert_eq!(diagnostic["start"]["character"], 7);
}

#[test]
fn draft_plan_preserves_missing_assets_and_directions_but_readiness_gate_fails() {
    let fixture = Fixture::new();
    let text = "scene harbor\n  shot: wide, dawn\n  mara: Hello\n";
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
        "wide, dawn"
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
    fixture.write(
        "story.sf",
        "scene harbor\n  mood: foreboding\n  Mara: Hello\n",
    );
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
fn trace_executes_real_runtime_and_rejects_corrupt_imported_plan() {
    let fixture = Fixture::new();
    fixture.write(
        "story.sf",
        "scene first\n  narrate: Hello\nscene second\n  narrate: World\n",
    );
    let compiled = fixture.json(&["story", "compile", "story.sf"], true);
    fixture.write("plan.json", &serde_json::to_string(&compiled).unwrap());
    fixture.write("commands.json", r#"["Advance","Advance"]"#);
    let events = fixture.json(&["story", "trace", "plan.json", "commands.json"], true);
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e.get("Completed").is_some()));
    let mut bad = compiled;
    bad["entry"] = serde_json::json!("missing");
    fixture.write("plan.json", &serde_json::to_string(&bad).unwrap());
    let output = fixture.run(&["story", "trace", "plan.json", "commands.json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid story plan"));
}

#[test]
fn workspace_plan_and_graph_diagnostics_keep_original_file_locations() {
    let fixture = Fixture::new();
    fixture.write("story.sf", "story \"Title\"\r\ninclude  \"chapter.sf\"\r\n");
    let report = fixture.json(&["story", "check", "story.sf"], false);
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["code"], "SF308");
    assert_eq!(diagnostic["file"], "story.sf");
    assert_eq!(diagnostic["start"], json!({"line": 1, "character": 0}));
    let chapter = "scene chapter\r\n  narrate: 🌊\r\n  end\r\n";
    fixture.write("chapter.sf", chapter);
    let plan = fixture.json(&["story", "compile", "story.sf"], true);
    assert_eq!(
        plan["source_map"]["workspace"]["files"]["chapter.sf"],
        chapter
    );
    fixture.write("plan.json", &plan.to_string());
    fixture.write("commands.json", r#"["Advance"]"#);
    std::fs::remove_file(fixture.dir.join("chapter.sf")).unwrap();
    let events = fixture.json(&["story", "trace", "plan.json", "commands.json"], true);
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event.get("Completed").is_some()));
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
