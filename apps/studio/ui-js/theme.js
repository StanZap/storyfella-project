// Appearance is independent of document/editor state and image pixels.
const element = document.documentElement;
element.dataset.themePreference = preference;
if (!window.storyfellaAppearance) {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
        const choice = element.dataset.themePreference;
        const theme = choice === 'system' ? (media.matches ? 'dark' : 'light') : choice;
        element.dataset.theme = theme;
        element.style.colorScheme = theme;
    };
    media.addEventListener('change', apply);
    window.storyfellaAppearance = {apply};
}
window.storyfellaAppearance.apply();
