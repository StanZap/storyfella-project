const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {test} = require('node:test');
const vm = require('node:vm');
const script = readFileSync(`${__dirname}/editable.js`, 'utf8');

function editor(tagName = 'INPUT') {
    const field = {tagName, disabled: false, value: 'native text not yet received by Rust'};
    const messages = [];
    let listener;
    const root = {
        querySelector: () => field,
        addEventListener: (name, callback) => { assert.equal(name, 'keydown'); listener = callback; },
    };
    vm.runInNewContext(script, {
        rootId: 'field', document: {getElementById: () => root},
        dioxus: {send: message => messages.push(JSON.parse(JSON.stringify(message)))},
    });
    return {
        field, messages,
        press(key, options = {}) {
            const event = {key, target: field, prevented: false, stopped: false,
                preventDefault() { this.prevented = true; },
                stopPropagation() { this.stopped = true; }, ...options};
            listener(event);
            return event;
        },
    };
}

test('Enter confirms the latest native value for both input types', () => {
    for (const tagName of ['INPUT', 'TEXTAREA']) {
        const e = editor(tagName);
        const key = e.press('Enter');
        assert.ok(key.prevented && key.stopped);
        assert.deepEqual(e.messages, [{action: 'confirm', value: e.field.value}]);
    }
});

test('Shift+Enter leaves multiline insertion and Tab traversal native', () => {
    const e = editor('TEXTAREA');
    for (const key of [e.press('Enter', {shiftKey: true}), e.press('Tab')]) {
        assert.equal(key.prevented, false);
    }
    assert.deepEqual(e.messages, []);
});

test('Escape discards from the field or its buttons without reaching navigation', () => {
    const e = editor();
    const key = e.press('Escape', {target: {tagName: 'BUTTON'}});
    assert.ok(key.prevented && key.stopped);
    assert.deepEqual(e.messages, [{action: 'cancel'}]);
    e.press('Enter', {target: {tagName: 'BUTTON'}});
    assert.equal(e.messages.length, 1, 'Enter on buttons retains native click activation');
});

test('composition, disabled controls and repeated keys do not confirm or discard', () => {
    const e = editor();
    e.press('Enter', {isComposing: true});
    e.press('Escape', {keyCode: 229});
    e.press('Enter', {repeat: true});
    e.field.disabled = true;
    e.press('Enter');
    e.press('Escape');
    assert.deepEqual(e.messages, []);
});
