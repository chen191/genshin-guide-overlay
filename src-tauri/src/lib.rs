mod episodes;
mod hotkeys;
mod media;
mod player;
mod settings;
mod windowing;

use hotkeys::{
    conflicts_with_fixed_shortcuts, normalize_shortcut_label, opacity_down_shortcut,
    opacity_up_shortcut, shortcut_from_label, OPACITY_DOWN_LABEL, OPACITY_UP_LABEL,
};
use serde::Serialize;
use settings::{
    clamp_zoom, guide_target_from_url, normalize_playback_rate, resolve_guide_input,
    resolve_guide_target, AppSettings, GuideMode, GuideTarget,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
    window::WindowBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

pub const CONTROL_HEIGHT: f64 = 44.0;

pub struct OverlayState {
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    locked: AtomicBool,
    current_target: Mutex<Option<GuideTarget>>,
    last_search: Mutex<Option<SearchSession>>,
    navigation_commit: Mutex<()>,
    navigation_generation: AtomicU64,
    media_generation: AtomicU64,
    hotkey_failures: Mutex<Vec<String>>,
}

#[derive(Debug, Clone)]
struct SearchSession {
    input: String,
    target: GuideTarget,
    results_json: String,
}

#[derive(Debug, Clone)]
enum SearchUpdate {
    Keep,
    Set(Box<SearchSession>),
    Clear,
}

#[derive(Debug)]
enum NavigationDecision {
    Allow,
    Deny,
    Intercept(GuideTarget),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayUiState {
    url: String,
    opacity: u8,
    zoom: u8,
    playback_rate: f64,
    play_hotkey: String,
    seek_backward_hotkey: String,
    seek_forward_hotkey: String,
    visibility_hotkey: String,
    lock_hotkey: String,
    locked: bool,
    video_only: bool,
    search_results: bool,
    last_search_input: Option<String>,
    hotkey_failures: Vec<String>,
}

fn snapshot_state(state: &OverlayState) -> OverlayUiState {
    let (
        saved_url,
        saved_search_input,
        opacity,
        configured_zoom,
        playback_rate,
        play_hotkey,
        seek_backward_hotkey,
        seek_forward_hotkey,
        visibility_hotkey,
        lock_hotkey,
    ) = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (
            settings.url.clone().unwrap_or_default(),
            settings.last_search_input.clone(),
            settings.opacity,
            settings.zoom,
            settings.playback_rate,
            settings.play_hotkey.clone(),
            settings.seek_backward_hotkey.clone(),
            settings.seek_forward_hotkey.clone(),
            settings.visibility_hotkey.clone(),
            settings.lock_hotkey.clone(),
        )
    };
    let current_target = state
        .current_target
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let last_search_input = state
        .last_search
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|search| search.input.clone())
        .or(saved_search_input);
    let video_only = current_target
        .as_ref()
        .is_some_and(GuideTarget::is_video_only);
    let search_results = current_target.as_ref().is_some_and(GuideTarget::is_search);
    let url = if search_results || (video_only && last_search_input.is_some()) {
        last_search_input.clone().unwrap_or(saved_url)
    } else {
        saved_url
    };
    OverlayUiState {
        url,
        opacity,
        zoom: current_target.as_ref().map_or(configured_zoom, |target| {
            target.effective_zoom(configured_zoom)
        }),
        playback_rate,
        play_hotkey,
        seek_backward_hotkey,
        seek_forward_hotkey,
        visibility_hotkey,
        lock_hotkey,
        locked: state.locked.load(Ordering::Acquire),
        video_only,
        search_results,
        last_search_input,
        hotkey_failures: state
            .hotkey_failures
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone(),
    }
}

pub(crate) fn emit_overlay_state(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let _ = app.emit_to("controls", "overlay-state", snapshot_state(&state));
}

#[tauri::command]
fn get_overlay_state(state: State<'_, OverlayState>) -> OverlayUiState {
    snapshot_state(&state)
}

#[tauri::command]
async fn navigate_content(app: AppHandle, input: String) -> Result<(), String> {
    let target = resolve_guide_input(&input)?;
    if target.is_search() {
        let input = target
            .source_url
            .query_pairs()
            .find_map(|(key, value)| (key == "keyword").then(|| value.into_owned()))
            .ok_or_else(|| "搜索关键词缺失".to_string())?;
        navigate_to_search(&app, input, target).await
    } else {
        navigate_to_target(&app, target, true, SearchUpdate::Clear)
    }
}

#[tauri::command]
async fn previous_episode(app: AppHandle) -> Result<String, String> {
    navigate_episode(app, EpisodeNavigationDirection::Previous).await
}

#[tauri::command]
async fn next_episode(app: AppHandle) -> Result<String, String> {
    navigate_episode(app, EpisodeNavigationDirection::Next).await
}

#[derive(Debug, Clone, Copy)]
enum EpisodeNavigationDirection {
    Previous,
    Next,
}

async fn navigate_episode(
    app: AppHandle,
    direction: EpisodeNavigationDirection,
) -> Result<String, String> {
    let current_target = app
        .state::<OverlayState>()
        .current_target
        .lock()
        .map_err(|_| "导航状态锁已损坏")?
        .clone()
        .ok_or_else(|| "请先打开一个 B 站攻略视频".to_string())?;
    if !current_target.is_video_only() {
        return Err("当前页面不是可切换分集的 B 站播放器".to_string());
    }

    let generation = next_navigation_generation(app.state::<OverlayState>().inner())?;
    let episode_result = match direction {
        EpisodeNavigationDirection::Previous => {
            episodes::fetch_previous_episode(&current_target).await
        }
        EpisodeNavigationDirection::Next => episodes::fetch_next_episode(&current_target).await,
    };
    if app
        .state::<OverlayState>()
        .navigation_generation
        .load(Ordering::Acquire)
        != generation
    {
        let label = match direction {
            EpisodeNavigationDirection::Previous => "上一集",
            EpisodeNavigationDirection::Next => "下一集",
        };
        return Ok(format!("已取消旧的{label}请求"));
    }
    let episode = episode_result?;

    let message = episode.message;
    navigate_to_target_generation(
        &app,
        guide_target_from_url(episode.source_url),
        true,
        SearchUpdate::Keep,
        generation,
    )?;
    Ok(message)
}

async fn navigate_to_search(
    app: &AppHandle,
    input: String,
    target: GuideTarget,
) -> Result<(), String> {
    let generation = next_navigation_generation(app.state::<OverlayState>().inner())?;
    let search_result = fetch_bilibili_search(&input).await;
    if app
        .state::<OverlayState>()
        .navigation_generation
        .load(Ordering::Acquire)
        != generation
    {
        return Ok(());
    }
    let results_json = search_result?;
    navigate_to_target_generation(
        app,
        target.clone(),
        false,
        SearchUpdate::Set(Box::new(SearchSession {
            input,
            target,
            results_json,
        })),
        generation,
    )
}

async fn fetch_bilibili_search(keyword: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("初始化 B 站搜索连接失败：{error}"))?;
    let response = client
        .get("https://api.bilibili.com/x/web-interface/wbi/search/type")
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131 Safari/537.36",
        )
        .header(reqwest::header::REFERER, "https://search.bilibili.com/")
        .query(&[
            ("search_type", "video"),
            ("keyword", keyword),
            ("page", "1"),
            ("order", "totalrank"),
        ])
        .send()
        .await
        .map_err(|error| format!("连接 B 站搜索失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "B 站搜索暂时不可用（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("读取 B 站搜索结果失败：{error}"))?;
    let code = payload.get("code").and_then(serde_json::Value::as_i64);
    if code != Some(0) {
        let message = payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("请稍后重试");
        return Err(format!("B 站搜索失败：{message}"));
    }
    serde_json::to_string(&payload).map_err(|error| format!("整理 B 站搜索结果失败：{error}"))
}

fn render_search_results(webview: &tauri::Webview, results_json: &str) -> Result<(), String> {
    let json_literal = serde_json::to_string(results_json).map_err(|error| error.to_string())?;
    let script = format!(
        "window.renderSearchResults && window.renderSearchResults(JSON.parse({json_literal}));"
    );
    webview.eval(&script).map_err(|error| error.to_string())
}

fn navigate_to_target(
    app: &AppHandle,
    target: GuideTarget,
    persist_url: bool,
    search_update: SearchUpdate,
) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    let generation = next_navigation_generation(state.inner())?;
    navigate_to_target_generation(app, target, persist_url, search_update, generation)
}

fn next_navigation_generation(state: &OverlayState) -> Result<u64, String> {
    Ok(state
        .navigation_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1))
}

fn navigate_to_target_generation(
    app: &AppHandle,
    target: GuideTarget,
    persist_url: bool,
    search_update: SearchUpdate,
    generation: u64,
) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    let _commit_guard = state
        .navigation_commit
        .lock()
        .map_err(|_| "导航提交锁已损坏")?;
    if state.navigation_generation.load(Ordering::Acquire) != generation {
        return Ok(());
    }
    media::invalidate_pending(app);
    let Some(content) = app.get_webview("content") else {
        return Err("攻略视图未就绪".to_string());
    };
    let search_input_update = match &search_update {
        SearchUpdate::Keep => None,
        SearchUpdate::Set(search) => Some(Some(search.input.clone())),
        SearchUpdate::Clear => Some(None),
    };
    let settings_changed = persist_url || search_input_update.is_some();
    let (previous_url, previous_search_input, configured_zoom) = {
        let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        let previous_url = settings.url.clone();
        let previous_search_input = settings.last_search_input.clone();
        let configured_zoom = settings.zoom;
        if persist_url {
            settings.url = Some(target.source_url.to_string());
        }
        if let Some(search_input) = search_input_update {
            settings.last_search_input = search_input;
        }
        if settings_changed {
            if let Err(error) = settings.save(&state.settings_path) {
                settings.url = previous_url;
                settings.last_search_input = previous_search_input;
                return Err(format!("保存导航状态失败：{error}"));
            }
        }
        (previous_url, previous_search_input, configured_zoom)
    };
    let previous_target = state
        .current_target
        .lock()
        .map_err(|_| "导航状态锁已损坏")?
        .replace(target.clone());
    let previous_search = {
        let mut last_search = state.last_search.lock().map_err(|_| "搜索状态锁已损坏")?;
        let previous = last_search.clone();
        match search_update {
            SearchUpdate::Keep => {}
            SearchUpdate::Set(search) => *last_search = Some(*search),
            SearchUpdate::Clear => *last_search = None,
        }
        previous
    };
    let previous_zoom = previous_target.as_ref().map_or(configured_zoom, |target| {
        target.effective_zoom(configured_zoom)
    });
    let target_zoom = target.effective_zoom(configured_zoom);
    if let Err(error) = content.set_zoom(f64::from(target_zoom) / 100.0) {
        let rollback_error = rollback_navigation_state(
            state.inner(),
            previous_target,
            previous_search,
            previous_url,
            previous_search_input,
            settings_changed,
        )?;
        return match rollback_error {
            Some(rollback_error) => Err(format!(
                "切换攻略缩放失败：{error}；恢复原网址也失败：{rollback_error}"
            )),
            None => Err(format!("切换攻略缩放失败：{error}")),
        };
    }
    let navigation = content
        .navigate(target.content_url.clone())
        .map_err(|error| error.to_string());
    if let Err(error) = navigation {
        let rollback_error = rollback_navigation_state(
            state.inner(),
            previous_target,
            previous_search,
            previous_url,
            previous_search_input,
            settings_changed,
        )?;
        let zoom_rollback_error = content
            .set_zoom(f64::from(previous_zoom) / 100.0)
            .err()
            .map(|error| error.to_string());
        let mut details = Vec::new();
        if let Some(rollback_error) = rollback_error {
            details.push(format!("恢复原网址失败：{rollback_error}"));
        }
        if let Some(rollback_error) = zoom_rollback_error {
            details.push(format!("恢复原缩放失败：{rollback_error}"));
        }
        return if details.is_empty() {
            Err(format!("打开攻略失败：{error}"))
        } else {
            Err(format!("打开攻略失败：{error}；{}", details.join("；")))
        };
    }
    emit_overlay_state(app);
    Ok(())
}

fn rollback_navigation_state(
    state: &OverlayState,
    previous_target: Option<GuideTarget>,
    previous_search: Option<SearchSession>,
    previous_url: Option<String>,
    previous_search_input: Option<String>,
    settings_changed: bool,
) -> Result<Option<String>, String> {
    *state
        .current_target
        .lock()
        .map_err(|_| "导航状态锁已损坏")? = previous_target;
    *state.last_search.lock().map_err(|_| "搜索状态锁已损坏")? = previous_search;
    if !settings_changed {
        return Ok(None);
    }
    let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
    settings.url = previous_url;
    settings.last_search_input = previous_search_input;
    Ok(settings.save(&state.settings_path).err())
}

fn navigation_decision(url: &url::Url) -> NavigationDecision {
    match url.scheme() {
        "tauri" => NavigationDecision::Allow,
        "http" | "https" => {
            let target = guide_target_from_url(url.clone());
            if target.is_video_only() && target.content_url != *url {
                NavigationDecision::Intercept(target)
            } else {
                NavigationDecision::Allow
            }
        }
        _ => NavigationDecision::Deny,
    }
}

fn popup_target_from_url(url: url::Url) -> GuideTarget {
    let target = guide_target_from_url(url.clone());
    if target.is_search() {
        GuideTarget {
            source_url: url.clone(),
            content_url: url,
            mode: GuideMode::Web,
        }
    } else {
        target
    }
}

fn defer_target_navigation(app: &AppHandle, target: GuideTarget) {
    let generation = match next_navigation_generation(app.state::<OverlayState>().inner()) {
        Ok(generation) => generation,
        Err(error) => {
            report_user_error(app, error);
            return;
        }
    };
    let dispatcher = app.clone();
    std::thread::spawn(move || {
        let queued_app = dispatcher.clone();
        if let Err(error) = dispatcher.run_on_main_thread(move || {
            if queued_app
                .state::<OverlayState>()
                .navigation_generation
                .load(Ordering::Acquire)
                != generation
            {
                return;
            }
            if let Err(error) = navigate_to_target_generation(
                &queued_app,
                target,
                true,
                SearchUpdate::Keep,
                generation,
            ) {
                report_user_error(&queued_app, format!("无法打开页面：{error}"));
            }
        }) {
            report_user_error(&dispatcher, format!("无法排队打开页面：{error}"));
        }
    });
}

#[tauri::command]
async fn return_to_search(app: AppHandle) -> Result<(), String> {
    let search = app
        .state::<OverlayState>()
        .last_search
        .lock()
        .map_err(|_| "搜索状态锁已损坏")?
        .clone();
    if let Some(search) = search {
        return navigate_to_target(
            &app,
            search.target.clone(),
            false,
            SearchUpdate::Set(Box::new(search)),
        );
    }

    let input = app
        .state::<OverlayState>()
        .settings
        .lock()
        .map_err(|_| "设置锁已损坏")?
        .last_search_input
        .clone()
        .ok_or_else(|| "当前没有可返回的上一层搜索".to_string())?;
    let target = resolve_guide_input(&input)?;
    if !target.is_search() {
        return Err("上一层搜索记录无效".to_string());
    }
    navigate_to_search(&app, input, target).await
}

#[tauri::command]
fn content_reload(app: AppHandle) -> Result<(), String> {
    media::invalidate_pending(&app);
    app.get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?
        .eval("location.reload()")
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn toggle_media(app: AppHandle) -> Result<(), String> {
    media::toggle_media(&app)
}

#[tauri::command]
fn seek_backward(app: AppHandle) -> Result<(), String> {
    media::seek_backward(&app)
}

#[tauri::command]
fn seek_forward(app: AppHandle) -> Result<(), String> {
    media::seek_forward(&app)
}

#[tauri::command]
fn show_quality(app: AppHandle) -> Result<(), String> {
    player::show_quality(&app)
}

#[tauri::command]
fn set_playback_rate(
    app: AppHandle,
    state: State<'_, OverlayState>,
    value: f64,
) -> Result<f64, String> {
    let value = normalize_playback_rate(value).ok_or_else(|| "不支持该播放速度".to_string())?;
    player::set_playback_rate(&app, value)?;

    let previous = {
        let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        let previous = settings.playback_rate;
        settings.playback_rate = value;
        if let Err(error) = settings.save(&state.settings_path) {
            settings.playback_rate = previous;
            let _ = player::set_playback_rate(&app, previous);
            return Err(format!("保存播放速度失败：{error}"));
        }
        previous
    };
    debug_assert!(normalize_playback_rate(previous).is_some());
    emit_overlay_state(&app);
    Ok(value)
}

#[tauri::command]
fn toggle_click_through(app: AppHandle, state: State<'_, OverlayState>) -> Result<bool, String> {
    let enabled = !state.locked.load(Ordering::Acquire);
    windowing::set_click_through(&app, enabled)?;
    Ok(enabled)
}

#[tauri::command]
fn set_opacity(app: AppHandle, value: u8) -> Result<u8, String> {
    windowing::set_opacity(&app, value)
}

#[tauri::command]
fn set_content_zoom(
    app: AppHandle,
    state: State<'_, OverlayState>,
    value: u8,
) -> Result<u8, String> {
    if let Some(fixed_zoom) = state
        .current_target
        .lock()
        .map_err(|_| "导航状态锁已损坏")?
        .as_ref()
        .filter(|target| target.is_video_only() || target.is_search())
        .map(|target| target.effective_zoom(value))
    {
        app.get_webview("content")
            .ok_or_else(|| "攻略视图未就绪".to_string())?
            .set_zoom(f64::from(fixed_zoom) / 100.0)
            .map_err(|error| error.to_string())?;
        return Ok(fixed_zoom);
    }
    let value = clamp_zoom(value);
    let content = app
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;
    let previous = state.settings.lock().map_err(|_| "设置锁已损坏")?.zoom;
    content
        .set_zoom(f64::from(value) / 100.0)
        .map_err(|error| error.to_string())?;
    let save_error = {
        let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        settings.zoom = value;
        match settings.save(&state.settings_path) {
            Ok(()) => None,
            Err(error) => {
                settings.zoom = previous;
                Some(error)
            }
        }
    };
    if let Some(error) = save_error {
        let result = match content.set_zoom(f64::from(previous) / 100.0) {
            Ok(()) => Err(format!("保存缩放比例失败：{error}")),
            Err(rollback_error) => Err(format!(
                "保存缩放比例失败：{error}；恢复原比例也失败：{rollback_error}"
            )),
        };
        emit_overlay_state(&app);
        return result;
    }
    emit_overlay_state(&app);
    Ok(value)
}

#[tauri::command]
fn open_shortcut_settings(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("shortcut-settings")
        .ok_or_else(|| "快捷键设置窗口尚未初始化".to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_hotkeys(
    app: AppHandle,
    state: State<'_, OverlayState>,
    play: String,
    seek_backward: String,
    seek_forward: String,
    visibility: String,
    lock: String,
) -> Result<(), String> {
    let normalize = |label: String| {
        normalize_shortcut_label(&label).ok_or_else(|| format!("不支持快捷键：{}", label.trim()))
    };
    let requested_labels = [
        normalize(play)?,
        normalize(seek_backward)?,
        normalize(seek_forward)?,
        normalize(visibility)?,
        normalize(lock)?,
    ];
    let requested_shortcuts = requested_labels
        .each_ref()
        .map(|label| shortcut_from_label(label).expect("normalized shortcut must be supported"));
    let has_duplicates = requested_shortcuts
        .iter()
        .enumerate()
        .any(|(index, shortcut)| requested_shortcuts[..index].contains(shortcut));
    if has_duplicates {
        return Err("五个功能不能使用同一个快捷键".to_string());
    }
    if requested_shortcuts
        .iter()
        .any(conflicts_with_fixed_shortcuts)
    {
        return Err("Ctrl+Alt+↑/↓ 已固定用于调整透明度".to_string());
    }

    let old_labels = {
        let settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        [
            settings.play_hotkey.clone(),
            settings.seek_backward_hotkey.clone(),
            settings.seek_forward_hotkey.clone(),
            settings.visibility_hotkey.clone(),
            settings.lock_hotkey.clone(),
        ]
    };
    let old_shortcuts = old_labels
        .each_ref()
        .map(|label| shortcut_from_label(label).expect("stored shortcut is validated on load"));
    let old_failures = state
        .hotkey_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let manager = app.global_shortcut();
    let mut unregistered_old: Vec<usize> = Vec::new();
    for (index, shortcut) in old_shortcuts.iter().enumerate() {
        if old_failures.contains(&old_labels[index]) {
            continue;
        }
        if let Err(error) = manager.unregister(*shortcut) {
            let mut rollback_errors = Vec::new();
            for restored in &unregistered_old {
                if let Err(rollback_error) = manager.register(old_shortcuts[*restored]) {
                    record_hotkey_failure(&state, old_labels[*restored].as_str());
                    rollback_errors.push(format!(
                        "恢复 {} 失败：{rollback_error}",
                        old_labels[*restored]
                    ));
                }
            }
            emit_overlay_state(&app);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("；{}", rollback_errors.join("；"))
            };
            return Err(format!(
                "无法注销旧快捷键 {}：{error}{suffix}",
                old_labels[index]
            ));
        }
        unregistered_old.push(index);
    }

    let mut registered_new: Vec<usize> = Vec::new();
    for (index, shortcut) in requested_shortcuts.iter().enumerate() {
        if let Err(error) = manager.register(*shortcut) {
            let mut rollback_errors = Vec::new();
            for registered in &registered_new {
                if let Err(rollback_error) = manager.unregister(requested_shortcuts[*registered]) {
                    record_hotkey_failure(&state, requested_labels[*registered].as_str());
                    rollback_errors.push(format!(
                        "注销 {} 失败：{rollback_error}",
                        requested_labels[*registered]
                    ));
                }
            }
            for restored in &unregistered_old {
                if let Err(rollback_error) = manager.register(old_shortcuts[*restored]) {
                    record_hotkey_failure(&state, old_labels[*restored].as_str());
                    rollback_errors.push(format!(
                        "恢复 {} 失败：{rollback_error}",
                        old_labels[*restored]
                    ));
                }
            }
            emit_overlay_state(&app);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("；{}", rollback_errors.join("；"))
            };
            return Err(format!(
                "{} 已被其他程序占用：{error}{suffix}",
                requested_labels[index]
            ));
        }
        registered_new.push(index);
    }

    let save_error = {
        let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        settings.play_hotkey = requested_labels[0].clone();
        settings.seek_backward_hotkey = requested_labels[1].clone();
        settings.seek_forward_hotkey = requested_labels[2].clone();
        settings.visibility_hotkey = requested_labels[3].clone();
        settings.lock_hotkey = requested_labels[4].clone();
        match settings.save(&state.settings_path) {
            Ok(()) => None,
            Err(error) => {
                settings.play_hotkey = old_labels[0].clone();
                settings.seek_backward_hotkey = old_labels[1].clone();
                settings.seek_forward_hotkey = old_labels[2].clone();
                settings.visibility_hotkey = old_labels[3].clone();
                settings.lock_hotkey = old_labels[4].clone();
                Some(error)
            }
        }
    };
    if let Some(error) = save_error {
        let mut rollback_errors = Vec::new();
        for registered in &registered_new {
            if let Err(rollback_error) = manager.unregister(requested_shortcuts[*registered]) {
                record_hotkey_failure(&state, requested_labels[*registered].as_str());
                rollback_errors.push(format!(
                    "注销 {} 失败：{rollback_error}",
                    requested_labels[*registered]
                ));
            }
        }
        for restored in &unregistered_old {
            if let Err(rollback_error) = manager.register(old_shortcuts[*restored]) {
                record_hotkey_failure(&state, old_labels[*restored].as_str());
                rollback_errors.push(format!(
                    "恢复 {} 失败：{rollback_error}",
                    old_labels[*restored]
                ));
            }
        }
        emit_overlay_state(&app);
        let suffix = if rollback_errors.is_empty() {
            String::new()
        } else {
            format!("；{}", rollback_errors.join("；"))
        };
        return Err(format!("保存快捷键失败：{error}{suffix}"));
    }

    for label in old_labels.iter().chain(requested_labels.iter()) {
        clear_hotkey_failure(&state, label);
    }
    emit_overlay_state(&app);
    Ok(())
}

fn record_hotkey_failure(state: &OverlayState, label: &str) {
    let mut failures = state
        .hotkey_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !failures.iter().any(|failure| failure == label) {
        failures.push(label.to_string());
    }
}

fn clear_hotkey_failure(state: &OverlayState, label: &str) {
    state
        .hotkey_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .retain(|failure| failure != label);
}

#[tauri::command]
fn hide_overlay(app: AppHandle) -> Result<(), String> {
    save_current_settings(&app)?;
    app.get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?
        .hide()
        .map_err(|error| error.to_string())
}

fn register_initial_hotkeys(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let core_labels = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        [
            settings.play_hotkey.clone(),
            settings.seek_backward_hotkey.clone(),
            settings.seek_forward_hotkey.clone(),
            settings.visibility_hotkey.clone(),
            settings.lock_hotkey.clone(),
        ]
    };
    let mut shortcuts: Vec<(String, Shortcut)> = core_labels
        .iter()
        .map(|label| {
            (
                label.clone(),
                shortcut_from_label(label).expect("validated hotkey"),
            )
        })
        .collect();
    shortcuts.push((OPACITY_UP_LABEL.to_string(), opacity_up_shortcut()));
    shortcuts.push((OPACITY_DOWN_LABEL.to_string(), opacity_down_shortcut()));
    let manager = app.global_shortcut();
    let mut failures = Vec::new();
    for (label, shortcut) in shortcuts {
        if manager.register(shortcut).is_err() {
            failures.push(label);
        }
    }
    *state
        .hotkey_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = failures;
}

fn handle_global_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let state = app.state::<OverlayState>();
    let core_labels = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        [
            settings.play_hotkey.clone(),
            settings.seek_backward_hotkey.clone(),
            settings.seek_forward_hotkey.clone(),
            settings.visibility_hotkey.clone(),
            settings.lock_hotkey.clone(),
        ]
    };
    if shortcut_from_label(&core_labels[0]).as_ref() == Some(shortcut) {
        let _ = media::toggle_media(app);
    } else if shortcut_from_label(&core_labels[1]).as_ref() == Some(shortcut) {
        let _ = media::seek_backward(app);
    } else if shortcut_from_label(&core_labels[2]).as_ref() == Some(shortcut) {
        let _ = media::seek_forward(app);
    } else if shortcut_from_label(&core_labels[3]).as_ref() == Some(shortcut) {
        let _ = windowing::toggle_visibility(app);
    } else if shortcut_from_label(&core_labels[4]).as_ref() == Some(shortcut) {
        let enabled = !state.locked.load(Ordering::Acquire);
        let _ = windowing::set_click_through(app, enabled);
    } else if shortcut == &opacity_up_shortcut() || shortcut == &opacity_down_shortcut() {
        let current = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .opacity;
        let next = if shortcut == &opacity_up_shortcut() {
            current.saturating_add(10)
        } else {
            current.saturating_sub(10)
        };
        let _ = windowing::set_opacity(app, next);
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示 / 隐藏", true, None::<&str>)?;
    let play = MenuItem::with_id(app, "play", "攻略播放 / 暂停", true, None::<&str>)?;
    let lock = MenuItem::with_id(app, "lock", "锁定 / 解锁鼠标穿透", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &play, &lock, &quit])?;
    TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "application icon is missing",
            ))
        })?)
        .menu(&menu)
        .tooltip("原神攻略悬浮窗")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = windowing::toggle_visibility(app);
            }
            "play" => {
                let _ = media::toggle_media(app);
            }
            "lock" => {
                let state = app.state::<OverlayState>();
                let enabled = !state.locked.load(Ordering::Acquire);
                let _ = windowing::set_click_through(app, enabled);
            }
            "quit" => {
                if let Err(error) = save_current_settings(app) {
                    report_settings_error(app, error);
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn save_current_settings(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let state = app.state::<OverlayState>();
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Ok(position) = window.outer_position() {
        let logical: tauri::LogicalPosition<f64> = position.to_logical(scale);
        settings.x = Some(logical.x);
        settings.y = Some(logical.y);
    }
    if let Ok(size) = window.inner_size() {
        let logical: tauri::LogicalSize<f64> = size.to_logical(scale);
        settings.width = logical.width;
        settings.height = logical.height;
    }
    settings.save(&state.settings_path)
}

fn report_settings_error(app: &AppHandle, error: String) {
    report_user_error(app, format!("设置保存失败：{error}"));
}

fn report_user_error(app: &AppHandle, message: String) {
    let _ = windowing::set_click_through(app, false);
    if let Some(window) = app.get_window("overlay") {
        let _ = window.show();
        let _ = window.set_always_on_top(true);
    }
    let _ = app.emit_to("controls", "settings-error", message);
}

fn restorable_saved_target(value: &str) -> Option<GuideTarget> {
    resolve_guide_target(value)
        .ok()
        .filter(|target| !target.is_search())
}

#[cfg(windows)]
fn show_fatal_error(message: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };

    let message: Vec<u16> = std::ffi::OsStr::new(message)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = std::ffi::OsStr::new("原神攻略悬浮窗")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_fatal_error(message: &str) {
    eprintln!("{message}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = windowing::set_click_through(app, false);
            if let Some(window) = app.get_window("overlay") {
                let _ = window.show();
                let _ = window.set_always_on_top(true);
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_global_shortcut)
                .build(),
        )
        .setup(|app| {
            let settings_path = app
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?
                .join("settings.json");
            let settings = AppSettings::load(&settings_path)?;
            let width = settings.width;
            let height = settings.height;
            let initial_x = settings.x;
            let initial_y = settings.y;
            let initial_opacity = settings.opacity;
            let configured_zoom = settings.zoom;
            let initial_target = settings.url.as_deref().and_then(restorable_saved_target);
            let initial_zoom = initial_target.as_ref().map_or(configured_zoom, |target| {
                target.effective_zoom(configured_zoom)
            });
            let content_url = initial_target
                .as_ref()
                .map(|target| WebviewUrl::External(target.content_url.clone()))
                .unwrap_or_else(|| WebviewUrl::App("welcome.html".into()));

            app.manage(OverlayState {
                settings: Mutex::new(settings),
                settings_path,
                locked: AtomicBool::new(false),
                current_target: Mutex::new(initial_target),
                last_search: Mutex::new(None),
                navigation_commit: Mutex::new(()),
                navigation_generation: AtomicU64::new(0),
                media_generation: AtomicU64::new(0),
                hotkey_failures: Mutex::new(Vec::new()),
            });

            let window = WindowBuilder::new(app, "overlay")
                .title("原神攻略悬浮窗")
                .inner_size(width, height)
                .min_inner_size(360.0, 220.0)
                .resizable(true)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .build()?;

            let (controls_position, controls_size, content_position, content_size) =
                windowing::initial_webview_bounds(width, height);
            window.add_child(
                WebviewBuilder::new("controls", WebviewUrl::App("index.html".into())),
                controls_position,
                controls_size,
            )?;

            let app_for_navigation = app.handle().clone();
            let app_for_popup = app.handle().clone();
            let content_builder = WebviewBuilder::new("content", content_url)
                .on_navigation(move |url| match navigation_decision(url) {
                    NavigationDecision::Allow => true,
                    NavigationDecision::Deny => false,
                    NavigationDecision::Intercept(target) => {
                        defer_target_navigation(&app_for_navigation, target);
                        false
                    }
                })
                .on_new_window(move |url, _features| {
                    if matches!(url.scheme(), "http" | "https") {
                        defer_target_navigation(&app_for_popup, popup_target_from_url(url.clone()));
                    }
                    NewWindowResponse::Deny
                })
                .on_page_load(|webview, payload| {
                    let event_url = payload.url().clone();
                    let finished = payload.event() == PageLoadEvent::Finished;
                    let app = webview.app_handle();
                    let state = app.state::<OverlayState>();
                    let current_target = state
                        .current_target
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    let Some(current_target) = current_target else {
                        return;
                    };
                    if current_target.content_url != event_url {
                        return;
                    }
                    if !finished {
                        media::invalidate_pending(app);
                    }
                    if finished {
                        let configured_zoom = state
                            .settings
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .zoom;
                        let zoom = current_target.effective_zoom(configured_zoom);
                        if let Err(error) = webview.set_zoom(f64::from(zoom) / 100.0) {
                            report_user_error(app, format!("恢复攻略页缩放失败：{error}"));
                        }
                        if current_target.is_search() {
                            let results_json = state
                                .last_search
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .as_ref()
                                .filter(|search| search.target.content_url == event_url)
                                .map(|search| search.results_json.clone());
                            if let Some(results_json) = results_json {
                                if let Err(error) = render_search_results(&webview, &results_json) {
                                    report_user_error(
                                        app,
                                        format!("显示 B 站搜索结果失败：{error}"),
                                    );
                                }
                            }
                        }
                        if current_target.is_video_only() {
                            if let Err(error) = player::apply_compact_layout(&webview) {
                                report_user_error(app, format!("精简播放器界面失败：{error}"));
                            }
                            let playback_rate = state
                                .settings
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .playback_rate;
                            if let Err(error) = player::apply_playback_rate(&webview, playback_rate)
                            {
                                report_user_error(app, format!("恢复播放速度失败：{error}"));
                            }
                        }
                        emit_overlay_state(app);
                    }
                    let display_url = snapshot_state(state.inner()).url;
                    windowing::emit_navigation_state(&webview, display_url, finished);
                });
            let content = window.add_child(content_builder, content_position, content_size)?;
            content.set_zoom(f64::from(initial_zoom) / 100.0)?;

            windowing::place_window(&window, initial_x, initial_y)
                .map_err(|error| error.to_string())?;
            windowing::apply_initial_opacity(app.handle(), initial_opacity)
                .map_err(|error| error.to_string())?;
            register_initial_hotkeys(app.handle());
            build_tray(app)?;
            window.show()?;
            emit_overlay_state(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "shortcut-settings" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            match event {
                WindowEvent::Resized(_) => {
                    let state = window.app_handle().state::<OverlayState>();
                    let locked = state.locked.load(Ordering::Acquire);
                    let _ = windowing::layout_webviews(window, locked);
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    let state = window.app_handle().state::<OverlayState>();
                    let locked = state.locked.load(Ordering::Acquire);
                    let _ = windowing::layout_webviews(window, locked);
                }
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    match save_current_settings(window.app_handle()) {
                        Ok(()) => {
                            let _ = window.hide();
                        }
                        Err(error) => report_settings_error(window.app_handle(), error),
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_overlay_state,
            navigate_content,
            previous_episode,
            next_episode,
            return_to_search,
            content_reload,
            toggle_media,
            seek_backward,
            seek_forward,
            show_quality,
            set_playback_rate,
            toggle_click_through,
            set_opacity,
            set_content_zoom,
            open_shortcut_settings,
            set_hotkeys,
            hide_overlay
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        show_fatal_error(&format!("原神攻略悬浮窗启动失败：{error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_bilibili_video_navigation_is_intercepted() {
        let url = url::Url::parse("https://www.bilibili.com/video/BV1oPDUYwEWD?p=2").unwrap();
        let NavigationDecision::Intercept(target) = navigation_decision(&url) else {
            panic!("standard video link should be intercepted");
        };
        assert!(target.is_video_only());
        assert_eq!(target.content_url.host_str(), Some("player.bilibili.com"));
    }

    #[test]
    fn player_and_search_navigation_are_allowed_without_a_loop() {
        for value in [
            "https://player.bilibili.com/player.html?bvid=BV1oPDUYwEWD&high_quality=1&autoplay=0&danmaku=0",
            "https://search.bilibili.com/all?keyword=%E5%8E%9F%E7%A5%9E",
            "tauri://localhost/welcome.html",
        ] {
            let url = url::Url::parse(value).unwrap();
            assert!(matches!(
                navigation_decision(&url),
                NavigationDecision::Allow
            ));
        }
    }

    #[test]
    fn unsafe_top_level_navigation_is_denied() {
        for value in [
            "file:///c:/secret.txt",
            "javascript:alert(1)",
            "data:text/plain,no",
        ] {
            let url = url::Url::parse(value).unwrap();
            assert!(matches!(
                navigation_decision(&url),
                NavigationDecision::Deny
            ));
        }
    }

    #[test]
    fn saved_search_page_waits_for_an_explicit_search_instead_of_spinning_forever() {
        assert!(restorable_saved_target(
            "https://search.bilibili.com/all?keyword=%E5%8E%9F%E7%A5%9E"
        )
        .is_none());
        assert!(restorable_saved_target("https://www.bilibili.com/video/BV1oPDUYwEWD").is_some());
    }

    #[test]
    fn search_popup_stays_a_regular_web_page_without_an_empty_local_result_view() {
        let url =
            url::Url::parse("https://search.bilibili.com/all?keyword=%E5%8E%9F%E7%A5%9E").unwrap();
        let target = popup_target_from_url(url.clone());
        assert_eq!(target.mode, GuideMode::Web);
        assert_eq!(target.content_url, url);
    }
}
