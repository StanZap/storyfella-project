// Mounted once per editable field. Native interception keeps Enter from adding
// a newline and Escape from navigating away before desktop Rust receives it.
const root = document.getElementById(rootId);
root.addEventListener('keydown', event => {
    const field = root.querySelector('input, textarea');
    if (!field || field.disabled || field.readOnly || event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        if (!event.repeat) dioxus.send({action: 'cancel'});
    } else if (event.key === 'Enter' && event.target === field) {
        if (field.tagName === 'TEXTAREA' && event.shiftKey) return;
        event.preventDefault();
        event.stopPropagation();
        if (!event.repeat) dioxus.send({action: 'confirm', value: field.value});
    }
});
