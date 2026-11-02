use dioxus::desktop::tao::window::Theme as NativeTheme;
use dioxus::prelude::*;
use storyfella_studio_core::config::ThemePreference;

#[component]
pub fn ThemeHost(preference: ThemePreference) -> Element {
    let window = dioxus::desktop::use_window();
    use_effect(use_reactive(&preference, move |preference| {
        window.window.set_theme(match preference {
            ThemePreference::Dark => Some(NativeTheme::Dark),
            ThemePreference::Light => Some(NativeTheme::Light),
            ThemePreference::System => None,
        });
        document::eval(&format!(
            "const preference = {:?};\n{}",
            preference.as_str(),
            include_str!("../../ui-js/theme.js")
        ));
    }));
    rsx! {}
}
