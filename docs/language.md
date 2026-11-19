# Storyfella language and editor services

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

A top-level `include "chapter.sf"` expands that file at the directive's
position. Paths are relative to the containing file, use forward slashes, and
stay inside the entry file's folder. Path components cannot be `.` or `..`.
Each file can be included once. Invalid, missing, duplicate, or cyclic includes
report SF308 at the responsible directive. Studio and configured LSP clients
combine unsaved buffers with saved files. The CLI reports diagnostics in the
authored files. Compiled workspace plans carry the exact source texts and the
mapping to their authored byte ranges, so playback can inspect them offline.

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

## Story flags and conditional flow

A story can declare named booleans at the top level of any included source.
Names are unique across the project. Every new playthrough starts from the
declared values. A scene or beat can set a flag, and a conditional can branch
on its value or its negation.

```storyfella
flag has_key false
scene entrance
  choice "Which route?"
    option "Take the key": take_key
    option "Go to the door": door
scene take_key
  set has_key true
  goto door
scene door
  if has_key goto inside else locked
scene inside
  narrate: The door opens.
  end
scene locked
  narrate: The door stays locked.
  end
```

`flag NAME true|false` declares the initial value. `set NAME true|false`
updates it immediately. `if [not] NAME goto SCENE else SCENE` checks the flag
and ends the scene with one of the two explicit jumps. Unknown or duplicate
flags and invalid scene targets are errors. An unknown scene (`SF202`) or flag
(`SF208`) names the declared one it was probably meant to be — "Unknown scene
'harbour'; did you mean 'harbor'?" — when exactly one is within a third of its
letters, ignoring case. Static reachability considers
both conditional targets. Flags survive scene changes and restore through
verified replay. Restart resets them to the declared values. Explicit scene
jumps keep the current values.

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

    document       = { blank-or-comment | story-header | include | flag | scene } ;
    story-header   = "story", quoted-text ;
    include        = "include", quoted-path ;
    flag           = "flag", NAME, BOOLEAN ;
    BOOLEAN        = "true" | "false" ;

    scene          = "scene", NAME, { scene-statement } ;
    scene-statement
                   = asset-statement | beat | choice | direction-statement
                   | speech-statement | narration-statement
                   | goto-statement | set-statement | if-statement | end-statement ;

    beat           = "beat", NAME, { beat-statement } ;
    beat-statement
                   = asset-statement | choice | direction-statement
                   | speech-statement | narration-statement
                   | goto-statement | set-statement | if-statement | end-statement ;

    asset-statement
                   = "cast", ":", NAME, { ",", NAME }
                   | "character", ":", NAME
                   | "location", ":", NAME
                   | "prop", ":", NAME
                   | "show", "illustration", NAME ;

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
    set-statement  = "set", NAME, BOOLEAN ;
    if-statement   = "if", [ "not" ], NAME, "goto", NAME, "else", NAME ;
    end-statement  = "end" ;

The grammar is indentation-sensitive. `scene` declarations occur only at the
top level alongside story titles, includes and flags. A title appears at most
once, before scenes. Beats occur only inside scenes. Options occur only
immediately inside a choice. Other statements occur inside a scene or beat.

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
    show illustration harbor_arrival

| Declaration | Required catalog kind | Meaning |
| --- | --- | --- |
| `cast` | Character | Declares one or more characters |
| `character` | Character | Declares one character |
| `location` | Location | Declares the scene environment |
| `prop` | Prop | Declares an object |
| `show illustration NAME` | Illustration | Shows the full illustration as the stage backdrop |

An illustration takes precedence over the location backdrop. It stays active until
another illustration is shown or playback enters another scene. Studio Preview
and Player fit the full image into the stage.

Declarations identify visual resources by name. Language analysis and compilation
do not create assets or invoke image generation. Studio reconciles valid explicit
declarations into pending shared catalog entries without images; unfinished or
invalid drafts retain the previous catalog. See [shared catalog](shared-project.md#shared-asset-catalog).

### Dialogue, thought, and narration

    mara: "You're late."
    mara think: She counts the minutes like coins.
    narrate: Rain taps against the glass.

Dialogue and thought use a character reference. Narration has no speaker and
does not require a character asset.

### Character staging

    scene harbor
      location: dock
      stage: lead, mara, left, 125, 4
      stage: friend, tam, right, 100, 2, 73, 12
      mara: Hello.
      swap: lead, mara_smile
      narrate: Mara smiles.
      hide: friend

`stage` takes an instance name, character artwork name, group (`left`, `center`,
`right`), scale percentage (25–300), and depth (0–999). Higher depth appears in
front. Optional `x, y` values (0–100) position the image's bottom center as a
percentage of the character stage: x from the left and y up from the bottom.
Use these to control spacing and overlap. Without them, members receive stable
automatic slots around their group anchor; adding or hiding another character
does not move them. Reusing an instance in `stage` updates its placement.
`swap` changes an existing instance's
artwork without changing its placement, size or depth; `hide` removes it. Both
require an instance already staged in that scene. Staging persists across dialogue
and beats and resets on entering a scene, including a jump back to that scene.
Save/restore reconstructs it by replay. Studio creates pending character catalog
entries for these artwork names; missing art is labeled in Preview. Prepare
swappable PNGs with matching dimensions and framing for consistent results.
Automatic variation compatibility checks are not yet supported.

### Directions

    style: watercolor at dusk
    shot: wide, low angle
    mood: uneasy but hopeful

The shared Preview and Player support these exact values (case-insensitive):

- `shot: wide` uses the normal framing; `shot: close` enlarges the backdrop.
- `mood: tense` adds a dark warm overlay; `mood: neutral` restores the default.
- `style: monochrome` removes colour from the backdrop, staged characters and portrait;
  `style: normal` restores colour.

Each effect lasts until replaced or the next scene begins. Restart and saved
state restore replay it deterministically. Other values remain in the plan and
block strict playback; Studio Preview skips them with a notice. These are
presentation effects on existing artwork, not new image generation.

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

1. The runtime has strict, allow-missing and Studio Preview policies.
2. Strict visual playback keeps current resource validation.
3. Allow-missing validation permits pending asset uses.
4. Unresolved asset uses remain available to Studio Preview as named placeholders
   without a fake revision; they do not enter the pinned visual set.
5. Preserve the authored speaker name and kind for dialogue when its artwork is
   absent; show the name without a portrait.
6. Render a neutral text stage when no visual is available. Text, thought,
   narration, choices, pause, restart, bookmarks, and source inspection must
   continue to work.
7. In text-only mode, skip `style`, `shot`, and `mood` only with a visible
   informational notice, or reject them with a precise diagnostic. Never imply
   that they affected presentation.
8. Studio Preview permits missing artwork and skips unsupported visual directions
   with a visible notice when the source is structurally valid. Placeholder
   unstaged placeholder positions are approximate.
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
Studio Preview can play it before the artwork is available.

## Explicitly unsupported in version 1

- values other than booleans, compound expressions, and dedicated loop syntax;
- localized text catalogs and locale selection;
- functions or macros;
- multiline blocks or heredocs;
- automatic timing or duration units;
- voice selection or speech synthesis annotations;
- audio, music, video, or transition declarations;
- asset definitions inside `.sf` files;
- `style`, `shot`, or `mood` values outside the documented visual effects;
- implicit asset creation or image generation.

Generators must not emit these constructs as if they were supported.

## Library contract

`parse(source)` returns original source, lossless tokens, ordered typed AST and
syntax diagnostics. Recovery proceeds at line boundaries. Trivia is in the token
stream; each AST text/name records its original span and decoded value. Spans are
half-open UTF-8 byte ranges, not displayed columns or audio positions.

`analyze(source, assets)` adds symbols, scoped references and diagnostics. The
host provides `{id, name, kind}` catalog entries. It does not query SQLite or
start inference. Missing assets are `SF203` informational pending references;
wrong-kind assets and ambiguous asset names are errors. Scene targets are checked
separately from external artwork, so a missing branch is an error even in drafts.

`definition_at`, `references_at`, `scope_at` and `complete` share that analysis.
Completion returns byte-span edits for keywords, scene targets and host-catalog
assets, with no completions inside prose or comments. `Source` translates byte
offsets to zero-based UTF-16 positions. Split surrogate pairs are rejected;
columns past end of a valid line clamp to its end.

## External editors

Build/run the server from the workspace:

```sh
cargo build -p storyfella-lsp
cargo run --quiet -p storyfella-lsp
```

For an editor, configure its generic LSP client with the executable
`<checkout>/target/debug/storyfella-lsp` (use the absolute checkout path),
file extension `.sf`, and language ID `storyfella`.
The server uses standard Content-Length-framed JSON-RPC on stdin/stdout; stderr
is for process failures. It follows the
[LSP 3.17 specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/).

Supported: initialize/shutdown/exit, open/change/close with incremental or full-document edits, versioned
published diagnostics, completion, definitions, references and hierarchical
scene/beat and flag document symbols. Position encoding is UTF-16. Older document versions
are ignored; incremental edits use UTF-16 ranges and are applied atomically.
Closing a standalone file clears its diagnostics. Closing a workspace file
returns its diagnostics to the saved source.

Resolved scene, beat and flag rename, prepare-rename, conservative formatting and full
semantic tokens are supported. Rename changes declarations and resolved targets,
rejects collisions, and leaves prose/comments untouched. Asset rename is rejected:
it needs a host operation that persists the catalog name. Writer's explicit Rename
also updates artwork association keys, retaining UUIDs; source-only editor renames
follow the documented detach/create behavior.

Export context with `sf-cli story catalog project.db > catalog.json`. Pass its
absolute path in `initializationOptions.catalogPath`. Entries contain `id`, `name`,
`kind` (`Character`, `Location`, `Prop`) and optional `revision`. An absent revision
means pending artwork (`SF306`); an absent entry means a missing asset (`SF203`).
Without a catalog references remain pending. Studio, CLI and LSP use the same
GUI-free catalog adapter. Refresh explicitly with
`workspace/didChangeConfiguration`, settings
`{"storyfella":{"catalogPath":"/absolute/catalog.json"}}`. Refresh reanalyzes
open unsaved buffers at their current versions. Failure preserves the previous
context and reports an error. There is no file watcher or source write, and no
inference startup. Compiler flow diagnostics share ordering and deduplication
with Writer and CLI.

Formatting edits whitespace gaps between structural tokens only. Indentation,
line endings, comments, quoted strings and raw prose are preserved; formatting
is idempotent. Syntax-invalid drafts are rejected without edits. Semantic tokens
cover keywords, text, comments, asset names and scene, beat and flag names.
External asset definition locations are outside the LSP catalog contract.

External LSP clients may supply an absolute `initializationOptions.entryPath` for a
project's entry `.sf` file. Completion, definitions, references, and symbol
rename then use the included workspace and open unsaved buffers. Go to Definition
on an include path opens its source file. Rename returns versioned edits for
open files and unversioned edits for closed files. Workspace diagnostics are
published against the original files, including closed files. Changes in any
open draft and save or catalog refresh events recompute them. The quick fix
for an unresolved scene target offers the exact spelling when just one
declaration matches by case. Formatting and semantic tokens operate per file.
Invalid include graphs report the include location and block workspace symbol edits.
