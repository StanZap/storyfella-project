// CodeMirror owns editor interaction and diff layout. Rust owns the draft,
// validation, acceptance, undo, persistence and model requests.
import {EditorState, Compartment, RangeSet, StateEffect, StateField, Transaction, ChangeSet} from "@codemirror/state";
import {EditorView, Decoration, GutterMarker, gutter, lineNumbers, keymap, drawSelection, highlightActiveLine} from "@codemirror/view";
import {defaultKeymap, history, historyField, historyKeymap} from "@codemirror/commands";
import {StreamLanguage, HighlightStyle, syntaxHighlighting} from "@codemirror/language";
import {tags} from "@lezer/highlight";
import {MergeView, unifiedMergeView, getChunks, getOriginalDoc, originalDocChangeEffect} from "@codemirror/merge";

// This is presentation-only token coloring, including deleted-line widgets.
// Full reference-aware highlighting comes from the shared Rust analysis.
const storyLanguage = StreamLanguage.define({
  startState: () => ({text: false}),
  token(stream, state) {
    if (stream.sol()) state.text = false;
    if (stream.eatSpace()) return null;
    if (stream.peek() === "#" && stream.string.slice(0, stream.pos).trim() === "") {
      stream.skipToEnd(); return "comment";
    }
    if (state.text) { stream.skipToEnd(); return "string"; }
    if (stream.match(/"(?:[^"\\]|\\.)*"?/)) return "string";
    if (stream.match(/(?:scene|beat|story|choice|option|goto|end|cast|character|location|prop|style|shot|mood|narrate|think)\b/)) return "keyword";
    if (stream.eat(":")) { state.text = true; return null; }
    if (stream.match(/[A-Za-z_][A-Za-z0-9_]*/)) return "variableName";
    stream.next(); return null;
  }
});
const colorStyle = HighlightStyle.define([
  {tag: tags.keyword, color: "#c4b5fd", fontWeight: "600"},
  {tag: tags.string, color: "#6ee7b7"},
  {tag: tags.comment, color: "#52525b", fontStyle: "italic"},
  {tag: tags.variableName, color: "#a1a1aa"}
]);
const tokenEffect = StateEffect.define();
const tokenField = StateField.define({
  create: () => Decoration.none,
  update(value, transaction) {
    value = value.map(transaction.changes);
    for (const effect of transaction.effects) if (effect.is(tokenEffect)) {
      value = Decoration.set(effect.value.filter(t => t.to > t.from && t.to <= transaction.newDoc.length)
        .map(t => Decoration.mark({class: t.class}).range(t.from, t.to)), true);
    }
    return value;
  },
  provide: field => EditorView.decorations.from(field)
});
const position = (doc, offset) => {
  offset = Math.max(0, Math.min(offset, doc.length));
  const line = doc.lineAt(offset);
  return {line: line.number - 1, character: offset - line.from};
};
const range = (doc, from, to) => ({start: position(doc, from), end: position(doc, to)});
const wholeChange = (view, text) => ({from: 0, to: view.state.doc.length, insert: text});

class OriginalLineNumber extends GutterMarker {
  constructor(number) { super(); this.number = number; }
  eq(other) { return other.number === this.number; }
  toDOM() { return document.createTextNode(String(this.number)); }
}
// Keep the two number columns beside the centre divider. A real gutter also
// follows wrapped lines and the merge view's alignment spacers.
const originalLineNumbers = gutter({
  class: "cm-lineNumbers",
  side: "after",
  lineMarker: (view, line) => new OriginalLineNumber(view.state.doc.lineAt(line.from).number),
  initialSpacer: view => new OriginalLineNumber(view.state.doc.lines),
  updateSpacer: (spacer, update) => spacer.number === update.state.doc.lines
    ? spacer : new OriginalLineNumber(update.state.doc.lines)
});

export function mount(parent, initial, send) {
  let config = initial, sequence = initial.sequence, selected = initial.selected;
  let marked = new Set(initial.marked);
  let merge = null, modified = null, suspended = true, locked = false, destroyed = false;
  const editable = new Compartment();
  const chunks = () => modified ? getChunks(modified.state)?.chunks ?? [] : [];
  const originalDoc = () => merge ? merge.a.state.doc : getOriginalDoc(modified.state);
  const scrollDOM = () => merge ? merge.dom : modified.scrollDOM;
  const emit = (kind, extra = {}) => {
    if (destroyed || !modified) return;
    const diff = chunks(), old = originalDoc(), doc = modified.state.doc;
    selected = Math.max(0, Math.min(selected, diff.length - 1));
    send({kind, session: initial.session, sequence, selected, marked: [...marked].sort((a, b) => a - b),
      text: doc.toString(), changes: diff.map(c => ({
        original: range(old, c.fromA, c.toA), proposed: range(doc, c.fromB, c.toB)
      })), ...extra});
  };
  const setEditable = () => {
    modified?.dispatch({effects: editable.reconfigure([
      EditorState.readOnly.of(config.disabled || locked), EditorView.editable.of(!config.disabled && !locked)
    ])});
    for (const control of parent.querySelectorAll('.studio-diff-gutter-actions button, .studio-diff-gutter-actions input')) {
      control.disabled = config.disabled || locked;
      if (control.type === "checkbox") control.checked = marked.has(Number(control.dataset.change));
    }
  };
  const act = (action, index = selected, all = false, group = false) => {
    if (locked || config.disabled || destroyed || group && !marked.size) return;
    selected = index;
    sequence++;
    locked = true;
    setEditable();
    emit("action", {action, all, group});
  };
  const navigate = direction => {
    if (locked || config.disabled || destroyed) return false;
    const index = selected + direction, diff = chunks();
    if (index < 0 || index >= diff.length) return false;
    selected = index;
    const anchor = Math.min(diff[index].fromB, modified.state.doc.length);
    modified.dispatch({selection: {anchor}, effects: EditorView.scrollIntoView(anchor, {y: "center"})});
    emit("selection");
    return true;
  };
  class Actions extends GutterMarker {
    constructor(index, kind) { super(); this.index = index; this.kind = kind; }
    eq(other) { return other.index === this.index && other.kind === this.kind; }
    toDOM() {
      const controls = document.createElement("span");
      controls.className = "studio-diff-gutter-actions";
      if (this.kind !== "select") {
        const reset = document.createElement("button");
        reset.textContent = "↶";
        reset.title = reset.ariaLabel = "Reset change " + (this.index + 1) + " to draft";
        reset.addEventListener("click", event => { event.preventDefault(); act("discard", this.index); });
        controls.append(reset);
      }
      if (this.kind !== "reset") {
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.checked = marked.has(this.index);
        checkbox.dataset.change = String(this.index);
        checkbox.title = checkbox.ariaLabel = "Select change " + (this.index + 1);
        checkbox.addEventListener("change", () => {
          if (checkbox.checked) marked.add(this.index); else marked.delete(this.index);
          emit("selection");
        });
        controls.append(checkbox);
      }
      return controls;
    }
  }
  const actionsGutter = (kind, original = false) => gutter({
    class: "studio-diff-actions studio-diff-actions-" + kind,
    side: original ? "after" : "before",
    markers(view) {
      const diff = getChunks(view.state)?.chunks ?? [];
      return RangeSet.of(diff.flatMap((c, i) => {
        // Pure additions have no original line. Put their reset control in
        // the alignment spacer, beside the first added line instead.
        if (original && c.fromA === c.toA) return [];
        const from = original ? c.fromA : c.fromB;
        return [new Actions(i, kind).range(view.state.doc.lineAt(Math.min(from, view.state.doc.length)).from)];
      }), true);
    },
    widgetMarker(view, widget, block) {
      if (!original) return null;
      const index = (getChunks(view.state)?.chunks ?? []).findIndex(c =>
        c.fromA === c.toA && Math.min(c.fromA, view.state.doc.length) === block.from);
      return index < 0 ? null : new Actions(index, kind);
    },
    lineMarkerChange: () => true
  });
  const listener = EditorView.updateListener.of(update => {
    if (suspended || !modified) return;
    if (update.docChanged || update.selectionSet) {
      const head = update.state.selection.main.head;
      const index = chunks().findIndex(c => c.fromB <= head && c.toB >= head);
      if (index >= 0) selected = index;
    }
    if (update.docChanged) {
      // Ranges can move or split while typing. Never apply an old selection
      // to a different set of changes after the diff is recalculated.
      marked.clear();
      sequence++;
      emit("change");
    } else if (update.selectionSet) {
      emit("selection");
    }
  });
  const base = (numbers = lineNumbers()) => [
    history(), keymap.of([...historyKeymap, ...defaultKeymap,
      {key: "F7", run: () => navigate(1)}, {key: "Shift-F7", run: () => navigate(-1)}]),
    numbers, drawSelection(), highlightActiveLine(), EditorView.lineWrapping,
    storyLanguage, syntaxHighlighting(colorStyle), tokenField, EditorState.tabSize.of(2),
    EditorView.theme({"&": {fontSize: "12.5px", color: "#a1a1aa", backgroundColor: "#0b0b0e"},
      ".cm-scroller": {fontFamily: "var(--font-mono, monospace)", lineHeight: "1.75"},
      ".cm-content": {padding: "12px 0"}, ".cm-line": {padding: "0 16px"},
      ".cm-cursor": {borderLeftColor: "#a78bfa"}, ".cm-activeLine": {backgroundColor: "#a78bfa0a"},
      ".cm-gutters": {backgroundColor: "#0b0b0e", color: "#52525b", border: "none"},
      ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {backgroundColor: "#a78bfa33"}
    }, {dark: true})
  ];
  const rebuild = mode => {
    const text = modified?.state.doc.toString() ?? config.proposed;
    const old = modified ? originalDoc().toString() : config.draft;
    const selection = modified?.state.selection;
    const undo = modified?.state.field(historyField, false);
    const scrollTop = modified ? scrollDOM().scrollTop : 0;
    const focused = modified?.hasFocus ?? false;
    suspended = true;
    if (merge) merge.destroy(); else modified?.destroy();
    parent.replaceChildren();
    merge = null;
    const extensions = [...base(), editable.of([]), actionsGutter(mode === "side_by_side" ? "select" : "both"), listener,
      EditorView.contentAttributes.of({"aria-label": "Suggested story", spellcheck: "false", autocapitalize: "off"})];
    if (undo) extensions.push(historyField.init(() => undo));
    if (mode === "side_by_side") {
      merge = new MergeView({
        a: {doc: old, extensions: [actionsGutter("reset", true), ...base(originalLineNumbers), EditorState.readOnly.of(true), EditorView.editable.of(false),
          EditorView.contentAttributes.of({"aria-label": "Original story"})]},
        b: {doc: text, selection, extensions},
        parent, highlightChanges: false, gutter: true, diffConfig: {scanLimit: 2000, timeout: 100}
      });
      modified = merge.b;
    } else {
      modified = new EditorView({parent, doc: text, selection,
        extensions: [...extensions, unifiedMergeView({original: old, highlightChanges: false,
          gutter: true, mergeControls: false, diffConfig: {scanLimit: 2000, timeout: 100}})]});
    }
    const scroller = scrollDOM();
    requestAnimationFrame(() => { if (scroller.isConnected) scroller.scrollTop = scrollTop; });
    // CodeMirror hides passive gutters from assistive technology. Our action
    // gutter contains real buttons, so expose it while keeping line numbers quiet.
    for (const editor of merge ? [merge.a, modified] : [modified]) {
      for (const gutters of editor.dom.querySelectorAll('.cm-gutters')) {
        if (!gutters.querySelector('.studio-diff-actions')) continue;
        gutters.removeAttribute('aria-hidden');
        for (const gutter of gutters.children) {
          if (!gutter.classList.contains('studio-diff-actions')) gutter.setAttribute('aria-hidden', 'true');
        }
      }
    }
    if (focused) modified.focus();
    setEditable();
    suspended = false;
  };
  const update = next => {
    if (destroyed) return;
    if (next.command) {
      if (next.command === "action") act(next.action, selected, next.all, next.group);
      else if (next.command === "retry") {
        if (!locked && !config.disabled) { sequence++; locked = true; setEditable(); emit("retry"); }
      }
      else if (next.command === "next") navigate(1);
      else if (next.command === "previous") navigate(-1);
      return;
    }
    const previousMode = config.mode;
    config = next;
    if (previousMode !== next.mode) rebuild(next.mode);
    if (next.sequence >= sequence) {
      sequence = next.sequence;
      locked = false;
      marked = new Set(next.marked);
      suspended = true;
      if (modified.state.doc.toString() !== next.proposed) {
        modified.dispatch({changes: wholeChange(modified, next.proposed),
          annotations: Transaction.addToHistory.of(false)});
      }
      if (originalDoc().toString() !== next.draft) {
        if (merge) merge.a.dispatch({changes: wholeChange(merge.a, next.draft),
          annotations: Transaction.addToHistory.of(false)});
        else {
          const original = originalDoc();
          // Use the public effect so CodeMirror retains its diff state.
          modified.dispatch({effects: originalDocChangeEffect(modified.state,
            ChangeSet.of({from: 0, to: original.length, insert: next.draft}, original.length))});
        }
      }
      modified.dispatch({effects: tokenEffect.of(next.proposed_tokens)});
      if (merge) merge.a.dispatch({effects: tokenEffect.of(next.draft_tokens)});
      suspended = false;
    }
    setEditable();
    queueMicrotask(() => emit("state"));
  };
  rebuild(initial.mode);
  update(initial);
  return {update, destroy() { destroyed = true; if (merge) merge.destroy(); else modified.destroy(); parent.replaceChildren(); }};
}

window.StoryfellaDiff = {mount};
