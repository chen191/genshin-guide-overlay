use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use url::Url;

pub const DEFAULT_WIDTH: f64 = 560.0;
pub const DEFAULT_HEIGHT: f64 = 350.0;
pub const DEFAULT_OPACITY: u8 = 90;
pub const MIN_OPACITY: u8 = 55;
pub const MAX_OPACITY: u8 = 100;
pub const DEFAULT_ZOOM: u8 = 40;
pub const MIN_ZOOM: u8 = 40;
pub const MAX_ZOOM: u8 = 100;
pub const SEARCH_ZOOM: u8 = 100;
pub const VIDEO_ONLY_ZOOM: u8 = 100;
pub const DEFAULT_PLAYBACK_RATE: f64 = 1.0;
pub const PLAYBACK_RATES: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideMode {
    Web,
    Search,
    Video,
}

#[derive(Debug, Clone)]
pub struct GuideTarget {
    pub source_url: Url,
    pub content_url: Url,
    pub mode: GuideMode,
}

impl GuideTarget {
    pub fn effective_zoom(&self, configured_zoom: u8) -> u8 {
        match self.mode {
            GuideMode::Web => clamp_zoom(configured_zoom),
            GuideMode::Search => SEARCH_ZOOM,
            GuideMode::Video => VIDEO_ONLY_ZOOM,
        }
    }

    pub fn is_video_only(&self) -> bool {
        self.mode == GuideMode::Video
    }

    pub fn is_search(&self) -> bool {
        self.mode == GuideMode::Search
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub url: Option<String>,
    pub last_search_input: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: f64,
    pub height: f64,
    pub opacity: u8,
    pub zoom: u8,
    pub playback_rate: f64,
    pub play_hotkey: String,
    pub seek_backward_hotkey: String,
    pub seek_forward_hotkey: String,
    pub visibility_hotkey: String,
    pub lock_hotkey: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            url: None,
            last_search_input: None,
            x: None,
            y: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            opacity: DEFAULT_OPACITY,
            zoom: DEFAULT_ZOOM,
            playback_rate: DEFAULT_PLAYBACK_RATE,
            play_hotkey: super::hotkeys::DEFAULT_PLAY_LABEL.to_string(),
            seek_backward_hotkey: super::hotkeys::DEFAULT_SEEK_BACKWARD_LABEL.to_string(),
            seek_forward_hotkey: super::hotkeys::DEFAULT_SEEK_FORWARD_LABEL.to_string(),
            visibility_hotkey: super::hotkeys::DEFAULT_VISIBILITY_LABEL.to_string(),
            lock_hotkey: super::hotkeys::DEFAULT_LOCK_LABEL.to_string(),
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!("无法读取设置文件 {}: {error}", path.display()));
            }
        };
        let mut settings: Self = match serde_json::from_str(&contents) {
            Ok(settings) => settings,
            Err(error) => {
                preserve_corrupt_settings(path)?;
                eprintln!("settings JSON was invalid and preserved as .json.corrupt: {error}");
                return Ok(Self::default());
            }
        };
        settings.width = settings.width.clamp(360.0, 1920.0);
        settings.height = settings.height.clamp(220.0, 1080.0);
        settings.opacity = clamp_opacity(settings.opacity);
        settings.zoom = clamp_zoom(settings.zoom);
        settings.playback_rate =
            normalize_playback_rate(settings.playback_rate).unwrap_or(DEFAULT_PLAYBACK_RATE);
        settings.last_search_input = settings
            .last_search_input
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        settings.play_hotkey = super::hotkeys::normalize_shortcut_label(&settings.play_hotkey)
            .unwrap_or_else(|| super::hotkeys::DEFAULT_PLAY_LABEL.to_string());
        settings.seek_backward_hotkey =
            super::hotkeys::normalize_shortcut_label(&settings.seek_backward_hotkey)
                .unwrap_or_else(|| super::hotkeys::DEFAULT_SEEK_BACKWARD_LABEL.to_string());
        settings.seek_forward_hotkey =
            super::hotkeys::normalize_shortcut_label(&settings.seek_forward_hotkey)
                .unwrap_or_else(|| super::hotkeys::DEFAULT_SEEK_FORWARD_LABEL.to_string());
        settings.visibility_hotkey =
            super::hotkeys::normalize_shortcut_label(&settings.visibility_hotkey)
                .unwrap_or_else(|| super::hotkeys::DEFAULT_VISIBILITY_LABEL.to_string());
        settings.lock_hotkey = super::hotkeys::normalize_shortcut_label(&settings.lock_hotkey)
            .unwrap_or_else(|| super::hotkeys::DEFAULT_LOCK_LABEL.to_string());
        let configured = [
            super::hotkeys::shortcut_from_label(&settings.play_hotkey),
            super::hotkeys::shortcut_from_label(&settings.seek_backward_hotkey),
            super::hotkeys::shortcut_from_label(&settings.seek_forward_hotkey),
            super::hotkeys::shortcut_from_label(&settings.visibility_hotkey),
            super::hotkeys::shortcut_from_label(&settings.lock_hotkey),
        ];
        let has_duplicates = configured
            .iter()
            .enumerate()
            .any(|(index, shortcut)| configured[..index].contains(shortcut));
        if has_duplicates
            || configured
                .iter()
                .flatten()
                .any(super::hotkeys::conflicts_with_fixed_shortcuts)
        {
            settings.play_hotkey = super::hotkeys::DEFAULT_PLAY_LABEL.to_string();
            settings.seek_backward_hotkey = super::hotkeys::DEFAULT_SEEK_BACKWARD_LABEL.to_string();
            settings.seek_forward_hotkey = super::hotkeys::DEFAULT_SEEK_FORWARD_LABEL.to_string();
            settings.visibility_hotkey = super::hotkeys::DEFAULT_VISIBILITY_LABEL.to_string();
            settings.lock_hotkey = super::hotkeys::DEFAULT_LOCK_LABEL.to_string();
        }
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let temporary = path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(contents.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        if let Err(error) = replace_settings_file(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok(())
    }
}

fn preserve_corrupt_settings(path: &Path) -> Result<(), String> {
    if path.is_file() {
        let backup = path.with_extension("json.corrupt");
        fs::copy(path, &backup).map_err(|error| {
            format!("设置文件已损坏，且无法备份到 {}: {error}", backup.display())
        })?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_settings_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
fn replace_settings_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(temporary, destination).map_err(|error| error.to_string())
}

pub fn clamp_opacity(value: u8) -> u8 {
    value.clamp(MIN_OPACITY, MAX_OPACITY)
}

pub fn clamp_zoom(value: u8) -> u8 {
    value.clamp(MIN_ZOOM, MAX_ZOOM)
}

pub fn normalize_playback_rate(value: f64) -> Option<f64> {
    PLAYBACK_RATES
        .into_iter()
        .find(|candidate| (value - candidate).abs() < 0.001)
}

pub fn normalize_external_url(input: &str) -> Result<Url, String> {
    let trimmed = clean_input(input);
    if trimmed.is_empty() {
        return Err("请先输入攻略网址".to_string());
    }
    let candidate = if explicit_scheme(trimmed).is_some() && !looks_like_trusted_bare_url(trimmed) {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&candidate).map_err(|_| "网址格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("仅支持 HTTP/HTTPS 攻略网址".to_string());
    }
    if url.host_str().is_none() {
        return Err("网址缺少站点域名".to_string());
    }
    Ok(url)
}

pub fn resolve_guide_target(input: &str) -> Result<GuideTarget, String> {
    Ok(guide_target_from_url(normalize_external_url(input)?))
}

pub fn resolve_guide_input(input: &str) -> Result<GuideTarget, String> {
    let trimmed = clean_input(input);
    if trimmed.is_empty() {
        return Err("请输入攻略关键词、BV/av 号或网址".to_string());
    }

    if let Some(video_url) = direct_bilibili_video_url(trimmed) {
        return Ok(guide_target_from_url(video_url));
    }

    if looks_like_trusted_bare_url(trimmed) {
        return resolve_guide_target(trimmed);
    }

    if let Some(scheme) = explicit_scheme(trimmed) {
        if matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
            return resolve_guide_target(trimmed);
        }
        return Err("仅支持 HTTP/HTTPS 攻略网址，不能打开本地文件或脚本协议".to_string());
    }

    let mut search_url = Url::parse("https://search.bilibili.com/all")
        .map_err(|_| "无法创建 B 站搜索地址".to_string())?;
    search_url.query_pairs_mut().append_pair("keyword", trimmed);
    let content_url = local_search_url(trimmed)?;
    Ok(GuideTarget {
        source_url: search_url,
        content_url,
        mode: GuideMode::Search,
    })
}

pub fn guide_target_from_url(source_url: Url) -> GuideTarget {
    if is_bilibili_player_url(&source_url) {
        return GuideTarget {
            content_url: player_url_with_defaults(source_url.clone()),
            source_url,
            mode: GuideMode::Video,
        };
    }

    if let Some(content_url) = bilibili_player_url(&source_url) {
        return GuideTarget {
            source_url,
            content_url,
            mode: GuideMode::Video,
        };
    }

    if let Some(keyword) = bilibili_search_keyword(&source_url) {
        if let Ok(content_url) = local_search_url(&keyword) {
            return GuideTarget {
                source_url,
                content_url,
                mode: GuideMode::Search,
            };
        }
    }

    GuideTarget {
        content_url: source_url.clone(),
        source_url,
        mode: GuideMode::Web,
    }
}

pub fn is_bilibili_player_url(url: &Url) -> bool {
    url.host_str() == Some("player.bilibili.com") && url.path() == "/player.html"
}

fn player_url_with_defaults(mut url: Url) -> Url {
    let existing: std::collections::HashSet<String> =
        url.query_pairs().map(|(key, _)| key.into_owned()).collect();
    let mut query = url.query_pairs_mut();
    for (key, value) in [("high_quality", "1"), ("autoplay", "0"), ("danmaku", "0")] {
        if !existing.contains(key) {
            query.append_pair(key, value);
        }
    }
    drop(query);
    url
}

fn clean_input(input: &str) -> &str {
    input.trim().trim_matches('"').trim()
}

fn explicit_scheme(input: &str) -> Option<&str> {
    let colon = input.find(':')?;
    let scheme = &input[..colon];
    let mut characters = scheme.chars();
    if !characters.next()?.is_ascii_alphabetic()
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
    {
        return None;
    }
    Some(scheme)
}

fn looks_like_trusted_bare_url(input: &str) -> bool {
    if input.contains("://") || input.chars().any(char::is_whitespace) {
        return false;
    }

    let authority = input.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(closing) = bracketed.find(']') else {
            return false;
        };
        let host = &bracketed[..closing];
        let remainder = &bracketed[closing + 1..];
        if !valid_optional_port(remainder) || host.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        host
    } else {
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if port.is_some_and(|port| !valid_port(port)) {
            return false;
        }
        host
    };

    host.parse::<std::net::IpAddr>().is_ok() || is_conservative_domain(host)
}

fn valid_optional_port(remainder: &str) -> bool {
    remainder.is_empty() || remainder.strip_prefix(':').is_some_and(valid_port)
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.chars().all(|character| character.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

fn is_conservative_domain(host: &str) -> bool {
    if !host.is_ascii() || host.len() > 253 || !host.contains('.') {
        return false;
    }

    let mut labels = host.split('.');
    let Some(last_label) = labels.next_back() else {
        return false;
    };
    if last_label.len() < 2
        || (!last_label
            .chars()
            .all(|character| character.is_ascii_alphabetic())
            && !last_label.to_ascii_lowercase().starts_with("xn--"))
    {
        return false;
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    })
}

fn direct_bilibili_video_url(input: &str) -> Option<Url> {
    let video_id = if is_complete_bvid(input) {
        format!("BV{}", &input[2..])
    } else if is_complete_avid(input) {
        format!("av{}", &input[2..])
    } else {
        return None;
    };
    Url::parse(&format!("https://www.bilibili.com/video/{video_id}")).ok()
}

fn is_complete_bvid(value: &str) -> bool {
    value.len() == 12
        && value
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("BV1"))
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn is_complete_avid(value: &str) -> bool {
    value
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("av"))
        && value.len() > 2
        && value[2..]
            .chars()
            .all(|character| character.is_ascii_digit())
        && value[2..].parse::<u64>().is_ok_and(|value| value > 0)
}

fn bilibili_search_keyword(url: &Url) -> Option<String> {
    if url.host_str() != Some("search.bilibili.com") {
        return None;
    }
    url.query_pairs()
        .find_map(|(key, value)| (key == "keyword").then(|| value.trim().to_string()))
        .filter(|value| !value.is_empty())
}

fn local_search_url(keyword: &str) -> Result<Url, String> {
    let mut url = Url::parse("http://tauri.localhost/search.html")
        .map_err(|_| "无法创建小窗搜索页".to_string())?;
    url.query_pairs_mut().append_pair("keyword", keyword);
    Ok(url)
}

fn bilibili_player_url(source: &Url) -> Option<Url> {
    let host = source.host_str()?;
    if !matches!(host, "bilibili.com" | "www.bilibili.com" | "m.bilibili.com") {
        return None;
    }

    let mut segments = source.path_segments()?;
    if !segments.next()?.eq_ignore_ascii_case("video") {
        return None;
    }
    let video_id = segments.next()?.trim();
    let prefix = video_id.get(..2)?;
    let (id_key, id_value) = if prefix.eq_ignore_ascii_case("BV") && is_complete_bvid(video_id) {
        ("bvid", video_id)
    } else if prefix.eq_ignore_ascii_case("av") && is_complete_avid(video_id) {
        ("aid", &video_id[2..])
    } else {
        return None;
    };

    let page = source
        .query_pairs()
        .find_map(|(key, value)| {
            (key == "p")
                .then(|| value.parse::<u32>().ok())
                .flatten()
                .filter(|value| *value > 0)
        })
        .unwrap_or(1)
        .to_string();

    let mut player = Url::parse("https://player.bilibili.com/player.html").ok()?;
    player
        .query_pairs_mut()
        .append_pair(id_key, id_value)
        .append_pair("page", &page)
        .append_pair("high_quality", "1")
        .append_pair("autoplay", "0")
        .append_pair("danmaku", "0");
    Some(player)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "genshin-guide-overlay-{name}-{}",
                std::process::id()
            ))
            .join("settings.json")
    }

    #[test]
    fn normalizes_bare_hosts_and_preserves_https() {
        assert_eq!(
            normalize_external_url("www.bilibili.com/video/BV1test")
                .unwrap()
                .as_str(),
            "https://www.bilibili.com/video/BV1test"
        );
        assert_eq!(
            normalize_external_url("https://example.com/guide?q=2")
                .unwrap()
                .as_str(),
            "https://example.com/guide?q=2"
        );
    }

    #[test]
    fn bilibili_video_links_use_the_pure_player() {
        let target = resolve_guide_target(
            "https://www.bilibili.com/video/BV1oPDUYwEWD/?p=3&vd_source=tracking",
        )
        .unwrap();

        assert!(target.is_video_only());
        assert_eq!(target.mode, GuideMode::Video);
        assert_eq!(target.effective_zoom(55), VIDEO_ONLY_ZOOM);
        assert_eq!(
            target.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD/?p=3&vd_source=tracking"
        );
        assert_eq!(target.content_url.host_str(), Some("player.bilibili.com"));
        let query: std::collections::HashMap<_, _> =
            target.content_url.query_pairs().into_owned().collect();
        assert_eq!(query.get("bvid").map(String::as_str), Some("BV1oPDUYwEWD"));
        assert_eq!(query.get("page").map(String::as_str), Some("3"));
        assert_eq!(query.get("high_quality").map(String::as_str), Some("1"));
        assert_eq!(query.get("autoplay").map(String::as_str), Some("0"));
        assert_eq!(query.get("danmaku").map(String::as_str), Some("0"));
    }

    #[test]
    fn older_settings_default_to_no_saved_search_input() {
        let settings: AppSettings = serde_json::from_str(r#"{"playHotkey":"F8"}"#).unwrap();
        assert!(settings.last_search_input.is_none());
        assert_eq!(settings.playback_rate, DEFAULT_PLAYBACK_RATE);
        assert_eq!(
            settings.seek_backward_hotkey,
            super::super::hotkeys::DEFAULT_SEEK_BACKWARD_LABEL
        );
        assert_eq!(
            settings.seek_forward_hotkey,
            super::super::hotkeys::DEFAULT_SEEK_FORWARD_LABEL
        );
        assert_eq!(
            settings.visibility_hotkey,
            super::super::hotkeys::DEFAULT_VISIBILITY_LABEL
        );
        assert_eq!(
            settings.lock_hotkey,
            super::super::hotkeys::DEFAULT_LOCK_LABEL
        );
    }

    #[test]
    fn bilibili_av_links_and_existing_players_are_video_only() {
        let av = resolve_guide_target("https://m.bilibili.com/video/av170001?p=0").unwrap();
        assert!(av.is_video_only());
        assert!(av.content_url.as_str().contains("aid=170001"));
        assert!(av.content_url.as_str().contains("page=1"));

        let player = resolve_guide_target(
            "https://player.bilibili.com/player.html?bvid=BV1oPDUYwEWD&page=2",
        )
        .unwrap();
        assert!(player.is_video_only());
        assert_eq!(player.source_url.host_str(), Some("player.bilibili.com"));
        let player_query: std::collections::HashMap<_, _> =
            player.content_url.query_pairs().into_owned().collect();
        assert_eq!(
            player_query.get("high_quality").map(String::as_str),
            Some("1")
        );
        assert_eq!(player_query.get("danmaku").map(String::as_str), Some("0"));
    }

    #[test]
    fn complete_bv_and_av_ids_open_the_pure_player_directly() {
        let bv = resolve_guide_input("  bv1oPDUYwEWD  ").unwrap();
        assert!(bv.is_video_only());
        assert_eq!(
            bv.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD"
        );
        let bv_query: std::collections::HashMap<_, _> =
            bv.content_url.query_pairs().into_owned().collect();
        assert_eq!(
            bv_query.get("bvid").map(String::as_str),
            Some("BV1oPDUYwEWD")
        );

        let av = resolve_guide_input("AV170001").unwrap();
        assert!(av.is_video_only());
        assert_eq!(
            av.source_url.as_str(),
            "https://www.bilibili.com/video/av170001"
        );
        let av_query: std::collections::HashMap<_, _> =
            av.content_url.query_pairs().into_owned().collect();
        assert_eq!(av_query.get("aid").map(String::as_str), Some("170001"));
    }

    #[test]
    fn keywords_build_a_bilibili_search_url_without_losing_special_characters() {
        let keyword = "原神 枫丹 & # + 跑图";
        let target = resolve_guide_input(&format!("  {keyword}  ")).unwrap();

        assert!(target.is_search());
        assert!(!target.is_video_only());
        assert_eq!(target.mode, GuideMode::Search);
        assert_eq!(target.source_url.host_str(), Some("search.bilibili.com"));
        assert_eq!(target.content_url.host_str(), Some("tauri.localhost"));
        assert_eq!(target.content_url.path(), "/search.html");
        assert_eq!(target.effective_zoom(40), SEARCH_ZOOM);
        assert_eq!(
            target
                .source_url
                .query_pairs()
                .find_map(|(key, value)| (key == "keyword").then(|| value.into_owned()))
                .as_deref(),
            Some(keyword)
        );
        assert_eq!(
            target
                .content_url
                .query_pairs()
                .find_map(|(key, value)| (key == "keyword").then(|| value.into_owned()))
                .as_deref(),
            Some(keyword)
        );
        assert!(!target.content_url.as_str().contains(" & "));
        assert!(!target.content_url.as_str().contains("# +"));
    }

    #[test]
    fn explicit_http_and_conservative_bare_domains_stay_urls() {
        for (input, expected) in [
            (
                "http://example.com/guide?q=原神",
                "http://example.com/guide?q=%E5%8E%9F%E7%A5%9E",
            ),
            (
                "www.bilibili.com/read/cv123",
                "https://www.bilibili.com/read/cv123",
            ),
            ("b23.tv/example", "https://b23.tv/example"),
            ("example.com:8443/guide", "https://example.com:8443/guide"),
        ] {
            let target = resolve_guide_input(input).unwrap();
            assert_eq!(target.mode, GuideMode::Web, "{input}");
            assert_eq!(target.source_url.as_str(), expected, "{input}");
            assert_eq!(target.source_url, target.content_url, "{input}");
        }
    }

    #[test]
    fn search_urls_restore_as_search_mode() {
        let target =
            resolve_guide_target("https://search.bilibili.com/all?keyword=%E5%8E%9F%E7%A5%9E")
                .unwrap();
        assert!(target.is_search());
        assert_eq!(target.effective_zoom(85), SEARCH_ZOOM);
        assert_eq!(target.content_url.host_str(), Some("tauri.localhost"));
        assert_eq!(target.content_url.path(), "/search.html");
    }

    #[test]
    fn bv_text_inside_a_keyword_is_not_mistaken_for_a_direct_id() {
        for input in ["原神 BV1oPDUYwEWD 跑图", "BV号怎么找", "BV1short", "av0"] {
            let target = resolve_guide_input(input).unwrap();
            assert!(target.is_search(), "{input}");
            assert_eq!(
                target
                    .content_url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "keyword").then(|| value.into_owned()))
                    .as_deref(),
                Some(input),
                "{input}"
            );
        }
    }

    #[test]
    fn guide_modes_choose_their_own_effective_zoom() {
        let web = resolve_guide_target("https://example.com/guide").unwrap();
        assert_eq!(web.effective_zoom(75), 75);
        assert_eq!(web.effective_zoom(1), MIN_ZOOM);

        let search = resolve_guide_input("原神攻略").unwrap();
        assert_eq!(search.effective_zoom(75), SEARCH_ZOOM);

        let video = resolve_guide_input("BV1oPDUYwEWD").unwrap();
        assert_eq!(video.effective_zoom(75), VIDEO_ONLY_ZOOM);
    }

    #[test]
    fn non_video_pages_keep_the_normal_web_view() {
        for input in [
            "https://www.bilibili.com/",
            "https://www.bilibili.com/read/cv123",
            "https://example.com/guide",
        ] {
            let target = resolve_guide_target(input).unwrap();
            assert!(!target.is_video_only());
            assert_eq!(target.mode, GuideMode::Web);
            assert_eq!(target.source_url, target.content_url);
        }
    }

    #[test]
    fn short_links_are_not_guessed_before_their_redirect() {
        let target = resolve_guide_target("https://b23.tv/example").unwrap();
        assert!(!target.is_video_only());
        assert_eq!(target.source_url, target.content_url);
    }

    #[test]
    fn rejects_empty_and_unsafe_schemes() {
        assert!(normalize_external_url(" ").is_err());
        assert!(normalize_external_url("javascript://alert(1)").is_err());
        assert!(normalize_external_url("file:///c:/secret.txt").is_err());

        for input in [
            "file:/c:/secret.txt",
            "file:///c:/secret.txt",
            "javascript:alert(1)",
            "javascript://alert(1)",
            "data:text/html,<h1>unsafe</h1>",
            "C:\\secret.txt",
        ] {
            assert!(resolve_guide_input(input).is_err(), "{input}");
        }
        assert!(resolve_guide_input("  ").is_err());
    }

    #[test]
    fn opacity_is_bounded() {
        assert_eq!(clamp_opacity(0), MIN_OPACITY);
        assert_eq!(clamp_opacity(90), 90);
        assert_eq!(clamp_opacity(255), MAX_OPACITY);
    }

    #[test]
    fn zoom_is_bounded() {
        assert_eq!(clamp_zoom(0), MIN_ZOOM);
        assert_eq!(clamp_zoom(DEFAULT_ZOOM), DEFAULT_ZOOM);
        assert_eq!(clamp_zoom(255), MAX_ZOOM);
    }

    #[test]
    fn playback_rate_only_accepts_supported_steps() {
        for rate in PLAYBACK_RATES {
            assert_eq!(normalize_playback_rate(rate), Some(rate));
        }
        assert_eq!(normalize_playback_rate(1.1), None);
        assert_eq!(normalize_playback_rate(f64::NAN), None);
    }

    #[test]
    fn duplicate_saved_shortcuts_reset_to_safe_defaults() {
        let path = test_settings_path("duplicate-hotkeys");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"playHotkey":"F8","visibilityHotkey":"F8","lockHotkey":"Ctrl+Alt+L"}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(
            loaded.play_hotkey,
            super::super::hotkeys::DEFAULT_PLAY_LABEL
        );
        assert_eq!(
            loaded.seek_backward_hotkey,
            super::super::hotkeys::DEFAULT_SEEK_BACKWARD_LABEL
        );
        assert_eq!(
            loaded.seek_forward_hotkey,
            super::super::hotkeys::DEFAULT_SEEK_FORWARD_LABEL
        );
        assert_eq!(
            loaded.visibility_hotkey,
            super::super::hotkeys::DEFAULT_VISIBILITY_LABEL
        );
        assert_eq!(
            loaded.lock_hotkey,
            super::super::hotkeys::DEFAULT_LOCK_LABEL
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn arbitrary_saved_shortcuts_are_canonicalized() {
        let path = test_settings_path("custom-hotkeys");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"playHotkey":"ctrl+shift+y","visibilityHotkey":"alt+win+num1","lockHotkey":"F19"}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.play_hotkey, "Ctrl+Shift+Y");
        assert_eq!(loaded.visibility_hotkey, "Alt+Win+Num1");
        assert_eq!(loaded.lock_hotkey, "F19");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn fixed_opacity_shortcuts_cannot_be_loaded_as_custom_actions() {
        let path = test_settings_path("fixed-hotkey-conflict");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"playHotkey":"Ctrl+Alt+Up","visibilityHotkey":"Ctrl+Alt+O","lockHotkey":"Ctrl+Alt+L"}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(
            loaded.play_hotkey,
            super::super::hotkeys::DEFAULT_PLAY_LABEL
        );
        assert_eq!(
            loaded.visibility_hotkey,
            super::super::hotkeys::DEFAULT_VISIBILITY_LABEL
        );
        assert_eq!(
            loaded.lock_hotkey,
            super::super::hotkeys::DEFAULT_LOCK_LABEL
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn settings_save_replaces_existing_file() {
        let path = test_settings_path("replace");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        let mut settings = AppSettings::default();
        settings.save(&path).unwrap();
        settings.zoom = 75;
        settings.play_hotkey = "F10".to_string();
        settings.save(&path).unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.zoom, 75);
        assert_eq!(loaded.play_hotkey, "F10");
        assert!(!path.with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_settings_are_preserved() {
        let path = test_settings_path("corrupt");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{broken").unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.zoom, DEFAULT_ZOOM);
        assert_eq!(
            fs::read_to_string(path.with_extension("json.corrupt")).unwrap(),
            "{broken"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn non_not_found_read_errors_are_reported() {
        let path = test_settings_path("read-error");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(&path).unwrap();

        assert!(AppSettings::load(&path).is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_backup_failures_are_reported() {
        let path = test_settings_path("backup-error");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{broken").unwrap();
        fs::create_dir_all(path.with_extension("json.corrupt")).unwrap();

        assert!(AppSettings::load(&path).is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
