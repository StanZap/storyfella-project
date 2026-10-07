# Storyfella language: initial contract

Status: the language front end, compiler, Writer adapter and initial stdio LSP are
implemented, including Studio `.sf` persistence and scene/beat registry associations.
Authoring tools and headless execution are implemented; visual hosts are next in
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

Resolved scene/beat rename, prepare-rename, conservative formatting and full
semantic tokens are supported. Rename changes declarations and resolved targets,
rejects collisions, and leaves prose/comments untouched. Asset rename is rejected:
it needs a host operation that persists the catalog name. Writer's explicit Rename
also updates artwork association keys, retaining UUIDs; source-only editor renames
follow the documented detach/create behavior.

Export context with `svs story catalog project.db > catalog.json`. Pass its
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
cover keywords, text, comments, asset names and scene/beat names. No imports,
external asset definition locations or code actions are advertised.

## Current boundaries

The [compiler](compiler.md) records sequential scene fallthrough, ordered beats,
and explicit branch/jump/end instructions; it does not execute choices, load voices
or generate assets. Expressions, variables, includes, speech annotations and
timing units remain unspecified. The [runtime](runtime.md) executes ready plans
with manual advancement, bounded flow and verified save/restore.
Studio reads and writes authoritative [`.sf` files](story-files.md); SQLite
stores the project's artwork and metadata. Language analysis itself remains
independent of storage.
