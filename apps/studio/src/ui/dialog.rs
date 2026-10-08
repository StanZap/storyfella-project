//! Native modal semantics: inert background, focus containment and Escape.
use dioxus::prelude::*;

#[component]
pub fn Dialog(title: String, on_close: EventHandler<()>, children: Element) -> Element {
    let id = use_hook(|| format!("dialog-{}", uuid::Uuid::new_v4()));
    let mount_id = id.clone();
    let cleanup_id = id.clone();
    use_drop(move || {
        document::eval(&format!(
            "const target = window.__dialogFocus?.['{cleanup_id}']; document.getElementById('{cleanup_id}')?.close(); delete window.__dialogFocus?.['{cleanup_id}']; requestAnimationFrame(() => {{ if (target?.isConnected) target.focus(); }});"
        ));
    });
    rsx! {
        dialog {
            id,
            class: "studio-dialog",
            aria_label: title,
            onmounted: move |_| {
                document::eval(&format!(
                    r#"window.__dialogFocus ??= {{}};
                    window.__dialogFocus['{mount_id}'] = document.activeElement;
                    const dialog = document.getElementById('{mount_id}');
                    dialog.showModal();
                    dialog.addEventListener('keydown', (event) => {{
                        if (event.key !== 'Tab') return;
                        const fields = [...dialog.querySelectorAll('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])')]
                            .filter(el => el.tabIndex >= 0 && el.getClientRects().length > 0);
                        const first = fields[0], last = fields[fields.length - 1];
                        if (!first) {{ event.preventDefault(); return; }}
                        if ((event.shiftKey && document.activeElement === first) ||
                            (!event.shiftKey && document.activeElement === last) ||
                            !fields.includes(document.activeElement)) {{
                            event.preventDefault();
                            (event.shiftKey ? last : first).focus();
                        }}
                    }}, true);"#
                ));
            },
            oncancel: move |event| {
                event.prevent_default();
                on_close.call(());
            },
            onkeydown: move |event| event.stop_propagation(),
            {children}
        }
    }
}
