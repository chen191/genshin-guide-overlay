fn main() {
    const COMMANDS: &[&str] = &[
        "get_overlay_state",
        "navigate_content",
        "previous_episode",
        "next_episode",
        "return_to_search",
        "content_reload",
        "toggle_media",
        "seek_backward",
        "seek_forward",
        "show_quality",
        "set_playback_rate",
        "toggle_click_through",
        "set_opacity",
        "set_content_zoom",
        "open_shortcut_settings",
        "set_hotkeys",
        "hide_overlay",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build Tauri application manifest");
}
