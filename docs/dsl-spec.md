# Storyfella Story DSL Specification

Version: 1 (current implementation)

This document is the authoring contract for Storyfella `.sf` files. It is
intended to be precise enough for an LLM or other generator to produce valid
stories without relying on Studio's UI.

The language is source-first. A story file describes narrative text and flow.
Artwork is supplied separately by Studio's project registry and is referenced
by name. A story may therefore be authored before its artwork exists. Studio
Preview can play such a story with black placeholders; standalone Player
enables the same behavior with `--allow-missing-assets`.

## File encoding and layout

- Source is UTF-8.
- LF and CRLF line endings are accepted and preserved.
- Indentation uses spaces only, exactly two spaces per level.
- Each statement occupies one physical line.
- Blank lines are allowed anywhere.
- A comment is a line whose first non-whitespace character is `#`.
- The file may end with or without a final newline.

Names are case-sensitive inside the story language. Asset lookup supplied by
Studio is case-insensitive, but catalog names that differ only by case are
ambiguous and invalid.

    NAME := [A-Za-z_][A-Za-z0-9_]*

## Lexical rules

Structural tokens are `:`, `,`, `"`, and `#`. Whitespace separates tokens;
whitespace inside quoted text is preserved.

Unquoted text extends to the end of the physical line. It may contain colons,
commas, `#`, punctuation, and backslashes. An unquoted `#` is prose; it starts
a comment only when it is the first non-whitespace character of the line.

Quoted text starts and ends with `"`. The only escapes are:

| Escape | Value |
| --- | --- |
| `\\\"` | quotation mark |
| `\\\\` | backslash |
| `\\n` | newline character in the decoded value |
| `\\t` | tab character in the decoded value |

Other escapes are errors. Text after a closing quote is an error.

## Grammar

The following grammar describes the accepted structure. `TEXT` means either
raw text to the end of the line or one quoted string, as required by the rule.

    document       = { blank-or-comment | story-header }, { scene } ;
    story-header   = "story", quoted-text ;

    scene          = "scene", NAME, { scene-statement } ;
    scene-statement
                   = asset-statement | beat | choice | direction-statement
                   | speech-statement | narration-statement
                   | goto-statement | end-statement ;

    beat           = "beat", NAME, { beat-statement } ;
    beat-statement
                   = asset-statement | choice | direction-statement
                   | speech-statement | narration-statement
                   | goto-statement | end-statement ;

    asset-statement
                   = "cast", ":", NAME, { ",", NAME }
                   | "character", ":", NAME
                   | "location", ":", NAME
                   | "prop", ":", NAME ;

    direction-statement
                   = "style", ":", TEXT
                   | "shot", ":", TEXT
                   | "mood", ":", TEXT ;

    speech-statement
                   = NAME, ":", TEXT
                   | NAME, "think", ":", TEXT ;

    narration-statement = "narrate", ":", TEXT ;
    choice         = "choice", quoted-text, { option } ;
    option          = "option", quoted-text, ":", NAME ;
    goto-statement = "goto", NAME ;
    end-statement  = "end" ;

The grammar is indentation-sensitive. `scene` declarations occur only at the
top level; `beat` declarations occur only inside scenes; `option`
declarations occur only immediately inside a choice. All other statements
occur inside a scene or beat.

The `story` header is optional, may occur at most once, and must precede every
scene. Its title must be quoted.

## Statements

### Story title

    story "The Lantern"

The title is presentation metadata and does not affect execution.

### Scenes and beats

    scene harbor
      beat arrival
        narrate: The boat reaches the pier.

The first scene is the entry point. Beats are named, ordered narrative groups;
they do not create a branch or a separate choice point. Beat names are unique
within their scene. Scene names are unique and define branch targets.

### Asset declarations

    cast: mara, tam
    character: mara
    location: harbor_dock
    prop: brass_lantern

| Declaration | Required catalog kind | Meaning |
| --- | --- | --- |
| `cast` | Character | Declares one or more characters |
| `character` | Character | Declares one character |
| `location` | Location | Declares the scene environment |
| `prop` | Prop | Declares an object |

Declarations identify visual resources by name. They do not create assets or
invoke image generation.

### Dialogue, thought, and narration

    mara: "You're late."
    mara think: She counts the minutes like coins.
    narrate: Rain taps against the glass.

Dialogue and thought use a character reference. Narration has no speaker and
does not require a character asset.

### Directions

    style: watercolor at dusk
    shot: wide, low angle
    mood: uneasy but hopeful

Directions are retained with source spans, but the current runtime does not
execute them. They currently make a plan not ready for strict visual playback.
Generators should omit them unless the target host explicitly supports them.

### Choices

    choice "Where should we go?"
      option "Follow the lantern": lighthouse
      option "Stay by the water": shore

A choice must contain at least two options. The prompt and every option label
must be quoted and nonempty. Labels must be distinct within the choice. Each
target must name a declared scene; forward references are allowed.

Selecting an option jumps to its target scene. There is no implicit return to
the statement after a choice.

### Goto and end

    goto lighthouse
    end

Both are terminal for their containing scene. `goto` immediately enters the
target scene; `end` completes the story.

## Execution semantics

1. Execution starts at the first scene.
2. Statements execute in source order.
3. A scene without `choice`, `goto`, or `end` falls through to the next
   declared scene.
4. The last scene without an explicit terminal receives an implicit `end`.
5. `choice`, `goto`, and `end` terminate the current scene.
6. Statements after a terminal are unreachable and are errors.
7. Cycles are allowed. Hosts apply a bounded transition budget.
8. Scenes unreachable from the entry scene produce warnings, not errors.

The runtime is manual and deterministic. It has no clock, sleep, audio device,
voice synthesis, or automatic text duration. A host explicitly advances text,
selects choices, pauses, resumes, jumps, restarts, or ends.

## Validation levels

1. **Syntax validity**: indentation, tokens, quoting, scopes, names, and
   statement shape.
2. **Story validity**: duplicate declarations, missing scenes, invalid flow,
   empty choices, and unreachable scenes.
3. **Resource readiness**: exact catalog identity, kind, revision, and usable
   artwork files.

Missing artwork is a pending resource requirement, not a syntax error. This is
intentional for source-first authoring and LLM generation.

## Asset-independent playback

The intended authoring flow is:

    write source -> validate story -> play text and choices
                   -> add artwork later -> play visually

The implementation uses an explicit playback capability rather than weakening
visual validation globally.

Playback contract:

1. The runtime has strict and allow-missing asset policies.
2. Strict visual playback keeps current resource validation.
3. Allow-missing validation permits pending asset uses.
4. Unresolved asset uses omit themselves from the active visual set rather
   than panic or require a fake revision.
5. Preserve the authored speaker name and kind for dialogue when its artwork is
   absent; show the name without a portrait.
6. Render a neutral text stage when no visual is available. Text, thought,
   narration, choices, pause, restart, bookmarks, and source inspection must
   continue to work.
7. In text-only mode, skip `style`, `shot`, and `mood` only with a visible
   informational notice, or reject them with a precise diagnostic. Never imply
   that they affected presentation.
8. Studio Preview starts with the allow-missing policy when the source is
   structurally valid but artwork is pending.
9. Player exposes `--allow-missing-assets`; in this mode it does not request an
   artwork project for compiled plans and uses placeholders instead.
10. Tests prove that the same source executes before assets exist
    and visually after exact revisions are added.

A useful UI label is:

    Text-only preview - 3 visual assets pending

No placeholder images or fake persisted assets should be created merely to pass
validation.

## LLM generation guidance

An LLM may generate dialogue speakers, locations, props, and casts as names
before those resources exist. The compiler should report them as pending and
text-only playback should remain available.

Until direction execution is implemented, generated stories should avoid
`style`, `shot`, and `mood`. Prefer short scenes, explicit terminals, and
unique choice labels. Generate only names matching `NAME`.

## Complete example

    story "The Last Ferry"

    scene pier
      narrate: The last ferry waited beneath a sky the color of iron.
      beat departure
        mara: "Are you coming?"
        tam think: The ticket trembles between her fingers.
        choice "What does Tam do?"
          option "Board the ferry": crossing
          option "Stay on the pier": return

    scene crossing
      narrate: The ferry pulled away before either of them spoke again.
      goto dawn

    scene return
      tam: "Let's go home."
      end

    scene dawn
      narrate: By morning, the opposite shore was already behind them.
      end

This is structurally valid even if `mara` and `tam` have no catalog entries.
It becomes playable text-only once asset-independent playback is implemented.

## Explicitly unsupported in version 1

- variables or expressions;
- conditions and loops;
- imports or includes;
- functions or macros;
- multiline blocks or heredocs;
- automatic timing or duration units;
- voice selection or speech synthesis annotations;
- audio, music, video, or transition declarations;
- asset definitions inside `.sf` files;
- executable semantics for `style`, `shot`, or `mood`;
- implicit asset creation or image generation.

Generators must not emit these constructs as if they were supported.
