use storyfella_language::*;

const STORY: &str = include_str!("../../../examples/harbor.sf");

#[test]
fn fixture_has_typed_structure_and_lossless_tokens() {
    let analysis = analyze(STORY, &[]);
    assert!(!analysis.has_errors(), "{:?}", analysis.diagnostics);
    let parsed = &analysis.document;
    assert_eq!(parsed.story.title.as_ref().unwrap().value, "The Lantern");
    assert_eq!(parsed.story.scenes.len(), 3);
    assert!(matches!(
        parsed.story.scenes[0].children[2].kind,
        NodeKind::Beat { .. }
    ));
    let rebuilt: String = parsed
        .tokens
        .iter()
        .map(|t| parsed.source.slice(t.span).unwrap())
        .collect();
    assert_eq!(rebuilt, STORY);
    let targets: Vec<_> = analysis
        .references
        .iter()
        .filter(|r| r.kind == ReferenceKind::Scene)
        .collect();
    assert_eq!(targets.len(), 3);
    assert!(targets.iter().all(|r| r.definition.is_some()));
    assert!(analysis
        .references
        .iter()
        .any(|r| r.name.value == "mara" && r.beat.as_deref() == Some("arrival")));
}

#[test]
fn unicode_and_crlf_positions_round_trip() {
    let text = "scene harbor\r\n  narrate: café 🌊 e\u{301}\r\n";
    let source = Source::new(text);
    for (offset, _) in text
        .char_indices()
        .filter(|(_, c)| !matches!(c, '\r' | '\n'))
    {
        assert_eq!(
            source.offset(source.position(offset).unwrap()),
            Some(offset)
        );
    }
    assert_eq!(
        source.offset(Position {
            line: 1,
            character: 17
        }),
        None,
        "cannot split an emoji surrogate pair"
    );
    assert_eq!(
        source.offset(Position {
            line: 0,
            character: 500
        }),
        Some(12)
    );
    assert_eq!(
        source.offset(Position {
            line: 50,
            character: 0
        }),
        None
    );
    let parsed = parse(text);
    assert_eq!(
        parsed
            .tokens
            .iter()
            .map(|t| source.slice(t.span).unwrap())
            .collect::<String>(),
        text
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

#[test]
fn raw_prose_keeps_colons_hashes_and_punctuation_and_quotes_decode() {
    let parsed = parse("story \"A \\\"title\\\"\"\nscene harbor\n  mara: Meet me: gate #3.\n  tam think: \"Line\\nnext\\t\\\\\"\n");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.story.title.unwrap().value, "A \"title\"");
    let children = &parsed.story.scenes[0].children;
    match &children[0].kind {
        NodeKind::Speech { text, .. } => assert_eq!(text.value, "Meet me: gate #3."),
        _ => panic!(),
    }
    match &children[1].kind {
        NodeKind::Speech { text, thought, .. } => {
            assert!(*thought);
            assert_eq!(text.value, "Line\nnext\t\\");
        }
        _ => panic!(),
    }
}

#[test]
fn recovery_keeps_later_scenes_after_multiple_errors() {
    assert!(parse("scene a\n\u{a0}mara: hello")
        .diagnostics
        .iter()
        .any(|d| d.code == "SF101"));
    let source = "scene bad\n   narrate: wrong indent\n\tprop: wrong\n  beat\n    mara: orphan\n  choice \"No options\"\nscene good\n  end\n";
    let analysis = analyze(source, &[]);
    assert!(analysis.has_errors());
    assert_eq!(analysis.document.story.scenes.len(), 2);
    assert_eq!(analysis.symbols.last().unwrap().name.value, "good");
    for code in ["SF101", "SF100", "SF102", "SF201"] {
        assert!(
            analysis.diagnostics.iter().any(|d| d.code == code),
            "missing {code}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn syntax_errors_are_local_and_do_not_panic() {
    for statement in [
        "cast:",
        "cast: mara,",
        "cast: mara tam",
        "location: a, b",
        "scene 123",
        "beat one:",
        "choice \"bad",
        "mara: \"bad\\q\"",
        "mara: \"two\" \"strings\"",
        "what is this",
        "goto",
        "end extra",
        "narrate:",
        "@old.sf",
        "mara: nul\0",
    ] {
        let text = format!("scene first\n  {statement}\nscene later\n  end\n");
        let parsed = parse(&text);
        assert!(!parsed.diagnostics.is_empty(), "accepted {statement}");
        assert!(parsed
            .story
            .scenes
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Scene { name } if name.value == "later")));
        for diagnostic in &parsed.diagnostics {
            assert!(parsed.source.slice(diagnostic.span).is_some());
        }
    }
}

#[test]
fn symbols_and_targets_are_unambiguous_and_beats_are_scene_local() {
    let analysis = analyze("scene a\n  beat shared\n    goto missing\n  beat shared\n    end\nscene b\n  beat shared\n    goto a\nscene a\n  end\n", &[]);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|d| d.code == "SF200")
            .count(),
        2
    );
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|d| d.code == "SF202")
            .count(),
        2
    );
    assert!(analysis.references.iter().all(|r| r.definition.is_none()));
}

#[test]
fn asset_resolution_checks_kind_and_case_insensitive_collisions() {
    let assets = [
        Asset {
            id: "char-1".into(),
            name: "Mara".into(),
            kind: AssetKind::Character,
        },
        Asset {
            id: "prop-1".into(),
            name: "lantern".into(),
            kind: AssetKind::Prop,
        },
    ];
    let analysis = analyze("scene harbor\n  cast: mara, tam\n  prop: lantern\n  location: lantern\n  mara: tam is a word in prose\n", &assets);
    assert_eq!(analysis.references.len(), 5);
    assert_eq!(analysis.references[0].asset_id.as_deref(), Some("char-1"));
    assert!(analysis
        .diagnostics
        .iter()
        .any(|d| d.code == "SF203" && d.severity == Severity::Information));
    assert!(analysis.diagnostics.iter().any(|d| d.code == "SF204"));
    let mut ambiguous = assets.to_vec();
    ambiguous.push(Asset {
        id: "char-2".into(),
        name: "mara".into(),
        kind: AssetKind::Character,
    });
    assert!(analyze("scene harbor\n  mara: hello", &ambiguous)
        .diagnostics
        .iter()
        .any(|d| d.code == "SF205"));
    assert!(
        !analyze("scene harbor\n  tam: hello", &[]).has_errors(),
        "pending assets cannot become syntax errors"
    );
}

#[test]
fn navigation_and_completion_share_source_spans() {
    let text = "scene start\n  goto finish\nscene finish\n  beat arrival\n    end\n";
    let analysis = analyze(text, &[]);
    let use_offset = text.find("finish").unwrap();
    let definition = analysis.definition_at(use_offset + 2).unwrap();
    assert_eq!(analysis.document.source.slice(definition), Some("finish"));
    assert_eq!(analysis.references_at(definition.start, true).len(), 2);
    assert_eq!(
        analysis.scope_at(text.find("end").unwrap()),
        (Some("finish"), Some("arrival"))
    );
    let completions = complete(&analysis, use_offset + 3, &[]);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].label, "finish");
    assert_eq!(
        analysis.document.source.slice(completions[0].replace),
        Some("finish")
    );
    assert!(complete(&analysis, text.len() + 1, &[]).is_empty());
    assert!(complete(&analyze("scene start\n  mara: fi", &[]), 22, &[]).is_empty());
}

#[test]
fn completion_respects_strings_container_types_and_catalog_kinds() {
    for source in [
        "scene start\n  choice \"Where?\"\n    option \"Label: fin",
        "scene start\n  mara: fin",
        "# scene fin",
    ] {
        let analysis = analyze(source, &[]);
        assert!(
            complete(&analysis, source.len(), &[]).is_empty(),
            "{source}"
        );
    }
    let source = "scene start\n  choice \"Where?\"\n    op";
    let analysis = analyze(source, &[]);
    let items = complete(&analysis, source.len(), &[]);
    assert_eq!(
        items.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["option"]
    );
    let assets = [
        Asset {
            id: "c".into(),
            name: "mara".into(),
            kind: AssetKind::Character,
        },
        Asset {
            id: "p".into(),
            name: "marble".into(),
            kind: AssetKind::Prop,
        },
    ];
    let source = "scene start\n  prop: ma";
    let analysis = analyze(source, &assets);
    let items = complete(&analysis, source.len(), &assets);
    assert_eq!(
        items.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["marble"]
    );
    let source = "scene start\n  beat arrival\n    be";
    let analysis = analyze(source, &assets);
    assert!(
        complete(&analysis, source.len(), &assets).is_empty(),
        "beats cannot nest"
    );
}

#[test]
fn tokens_cover_arbitrary_utf8_without_gaps() {
    // Regression corpus includes unfinished edits, CRLF, escapes and punctuation.
    for source in [
        "",
        "\n",
        "\r\n",
        "# 🌊",
        "a\r",
        "\u{feff}scene",
        "scene café\n",
        "  mara: \"a\\\"\"",
        "scene a\n  choice \"x:y\"\n    option \"a:b\": c\n",
        "\t\0🙂\n",
    ] {
        let parsed = parse(source);
        let mut end = 0;
        for token in &parsed.tokens {
            assert_eq!(token.span.start, end, "gap in {source:?}");
            end = token.span.end;
        }
        assert_eq!(end, source.len());
        assert_eq!(
            parsed
                .tokens
                .iter()
                .map(|t| parsed.source.slice(t.span).unwrap())
                .collect::<String>(),
            source
        );
    }
}
