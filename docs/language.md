# Storyfella language: initial contract

Status: the language front end, compiler, Writer adapter and initial stdio LSP are
implemented, including Studio `.sf` persistence and scene/beat registry associations.
Authoring tools and playback are the next milestones in
[the execution plan](language-plan.md). This syntax is a first editable design,
anchored to the current Writer; it is not a compatibility layer for the prototype.

## Source and syntax

UTF-8 `.sf` source uses two spaces per indentation level. Blank lines and comments
whose first non-whitespace character is `#` are trivia. CRLF and LF are preserved.
Block headers have no trailing colon. Names begin with an ASCII letter or `_`,
followed by ASCII letters, digits or `_`. Scene/beat names are case-sensitive;
external asset lookup is case-insensitive and rejects collisions.

```sf
story "The Lantern"

scene harbor
  location: harbor_dock
  cast: mara, tam

  beat arrival
    shot: wide, dawn
    mara: "You're late. Tide's already turning."
    tam think: She counts the minutes like coins.
    choice "Where should we go?"
      option "Follow the lantern": lighthouse
      option "Stay by the water": shore

scene lighthouse
  prop: brass_lantern
  narrate: Rain taps against the glass.
  end

scene shore
  goto lighthouse
```

The optional `story "title"` appears once before scenes. A document may be empty
or contain incomplete scenes while editing. Only scenes occur at the top level.
Beats occur inside scenes. Statements occur inside scenes or beats; a choice has
only options. Choices require at least two options. Option and `goto` targets name
scenes anywhere in this document, including forward references. Beat names are
local to their scene. Duplicate declarations and missing targets are errors.

| Statement | Parsed intent |
| --- | --- |
| `cast: mara, tam` | List of character assets |
| `character: mara` | One character asset |
| `location: harbor_dock` | One location/environment asset |
| `prop: brass_lantern` | One prop/object asset |
| `mara: text` | Dialogue with a character reference |
| `mara think: text` | Thought with a character reference |
| `narrate: text` | Narration with no invented character asset |
| `style: text`, `shot: text`, `mood: text` | Authored directions; execution semantics deferred |
| `choice "prompt"` / `option "label": target` | Branch prompt and scene destinations |
| `goto target`, `end` | Terminal scene jump or story ending, lowered by the compiler |

Prose is either raw text to end of line, or one quoted string. Raw text preserves
colons, `#`, punctuation and literal backslashes. It is not scanned for asset names.
Quoted text accepts `\"`, `\\`, `\n` and `\t`; other escapes and trailing text are
errors. Titles, choice prompts and option labels require quoted strings. Each
statement occupies one source line; multiline blocks have not been designed yet.

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
`/tmp/storyfella-studio-target/debug/storyfella-lsp` (the repository's configured
build target directory), file extension `.sf`, and language ID `storyfella`.
The server uses standard Content-Length-framed JSON-RPC on stdin/stdout; stderr
is for process failures. It follows the
[LSP 3.17 specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/).

Supported: initialize/shutdown/exit, full-document open/change/close, versioned
published diagnostics, completion, definitions, references and hierarchical
scene/beat document symbols. Position encoding is UTF-16. Older document versions
are ignored; incremental changes are rejected because the server advertises full
sync. Closing a file clears published diagnostics.

The server currently analyzes each open document independently with an empty
asset catalog. External asset references therefore show as pending; Studio alone
supplies its live registry. No workspace imports, external asset definitions,
rename, formatter, semantic tokens or runtime integration are advertised yet.

## Current boundaries

The [compiler](compiler.md) records sequential scene fallthrough, ordered beats,
and explicit branch/jump/end instructions; it does not execute choices, load voices
or generate assets. Expressions, variables, includes, speech annotations and
timing units remain unspecified. No placeholder runtime crate exists.
Studio reads and writes authoritative [`.sf` files](story-files.md); SQLite
stores the project's artwork and metadata. Language analysis itself remains
independent of storage.
