use storyfella_language::*;

#[test]
fn resolved_rename_preserves_prose_comments_and_unrelated_scopes() {
    let source = "# harbor stays in this comment 🌊\r\nscene harbor\r\n  beat harbor\r\n    narrate: harbor: # harbor\r\n  goto sea\r\nscene sea\r\n  goto harbor\r\n";
    let analysis = analyze(source, &[]);
    let edits = rename(&analysis, source.find("scene harbor").unwrap() + 6, "port").unwrap();
    let renamed = apply_edits(source, &edits).unwrap();
    assert!(renamed.contains("scene port\r\n"));
    assert!(renamed.ends_with("  goto port\r\n"));
    assert!(renamed.contains("  beat harbor\r\n    narrate: harbor: # harbor\r\n"));
    assert!(renamed.starts_with("# harbor stays in this comment 🌊\r\n"));
    assert!(rename(&analysis, source.find("scene harbor").unwrap() + 6, "sea").is_err());
    assert!(rename(&analysis, 0, "9bad").is_err());
    let unresolved = analyze("scene a\n  goto absent\n", &[]);
    assert!(rename(&unresolved, 16, "b").is_err());
}

#[test]
fn asset_rename_is_rejected_and_asset_references_share_identity() {
    let source = "scene a\n  cast: Mara\n  mara: Hello\n";
    let analysis = analyze(
        source,
        &[Asset {
            id: "mara-id".into(),
            name: "mara".into(),
            kind: AssetKind::Character,
        }],
    );
    let offset = source.find("Mara").unwrap();
    assert_eq!(analysis.references_at(offset, true).len(), 2);
    assert!(rename(&analysis, offset, "tam")
        .unwrap_err()
        .contains("host catalog"));
}

#[test]
fn formatting_is_idempotent_and_preserves_prose_and_line_endings() {
    let source = "# comment  unchanged 🌊\r\nstory   \"A  title\"\r\nscene   harbor\r\n  cast : mara , tam\r\n  beat   arrival\r\n    mara : Raw  prose: # stays  \r\n    narrate: \"More  words\"\r\n";
    let analysis = analyze(source, &[]);
    assert!(
        analysis.document.diagnostics.is_empty(),
        "{:?}",
        analysis.document.diagnostics
    );
    let formatted = apply_edits(source, &format(&analysis).unwrap()).unwrap();
    assert!(formatted.contains("scene harbor\r\n  cast: mara, tam\r\n"));
    assert!(formatted.contains("mara: Raw  prose: # stays  \r\n"));
    assert!(formatted.contains("\"A  title\""));
    assert!(formatted.starts_with("# comment  unchanged 🌊\r\n"));
    assert!(format(&analyze(&formatted, &[])).unwrap().is_empty());
    assert!(format(&analyze("scene a\n   broken\n", &[])).is_err());
}

#[test]
fn text_edits_and_textarea_updates_preserve_unicode_and_mixed_endings() {
    let original = "# 🌊\r\nscene harbor\n  narrate: café\r\n";
    assert_eq!(
        textarea_input(original, "# 🌊\nscene harbor\n  narrate: café!\n"),
        "# 🌊\r\nscene harbor\n  narrate: café!\r\n"
    );
    assert_eq!(
        textarea_input(original, &original.replace("\r\n", "\n")),
        original
    );
    assert!(apply_edits(
        original,
        &[TextEdit {
            replace: Span::new(3, 4),
            new_text: "x".into()
        }]
    )
    .is_err());
    assert!(apply_edits(
        "abc",
        &[
            TextEdit {
                replace: Span::new(0, 2),
                new_text: "x".into()
            },
            TextEdit {
                replace: Span::new(1, 3),
                new_text: "y".into()
            }
        ]
    )
    .is_err());
}

#[test]
fn semantic_spans_are_valid_utf16_ranges_and_opaque_prose_is_not_a_symbol() {
    let analysis = analyze("# 🌊\nscene harbor\n  mara: 🌊 harbor\n", &[]);
    let tokens = semantic_tokens(&analysis);
    let prose = tokens
        .iter()
        .find(|t| analysis.document.source.slice(t.span) == Some("🌊 harbor"))
        .unwrap();
    assert_eq!(prose.kind, 1);
    let start = analysis.document.source.position(prose.span.start).unwrap();
    let end = analysis.document.source.position(prose.span.end).unwrap();
    assert_eq!(end.character - start.character, 9);
    assert!(tokens
        .iter()
        .all(|token| analysis.document.source.slice(token.span).is_some()));
}
