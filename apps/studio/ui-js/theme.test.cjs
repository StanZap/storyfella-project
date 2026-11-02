const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {test} = require('node:test');
const vm = require('node:vm');
const script = readFileSync(`${__dirname}/theme.js`, 'utf8');

test('appearance follows live system changes only in System mode, with one listener', () => {
    const element = {dataset: {}, style: {}};
    let listener, registrations = 0;
    const media = {
        matches: false,
        addEventListener(name, fn) {
            assert.equal(name, 'change');
            listener = fn;
            registrations++;
        },
    };
    const context = vm.createContext({document: {documentElement: element}, window: {
        matchMedia: query => { assert.equal(query, '(prefers-color-scheme: dark)'); return media; },
    }});
    const choose = preference => vm.runInContext(
        `(function() {const preference = ${JSON.stringify(preference)};\n${script}\n})()`, context,
    );
    choose('system');
    assert.equal(element.dataset.theme, 'light');
    media.matches = true;
    listener();
    assert.equal(element.dataset.theme, 'dark');
    assert.equal(element.style.colorScheme, 'dark');
    choose('light');
    listener();
    assert.equal(element.dataset.theme, 'light');
    choose('dark');
    media.matches = false;
    listener();
    assert.equal(element.dataset.theme, 'dark');
    choose('system');
    assert.equal(element.dataset.theme, 'light');
    assert.equal(registrations, 1);
});
