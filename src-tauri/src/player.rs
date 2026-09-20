use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const COMPACT_PLAYER_SCRIPT: &str = r#"
(() => {
  if (location.host !== 'player.bilibili.com') return false;

  const styleId = 'genshin-guide-compact-player';
  let style = document.getElementById(styleId);
  if (!style) {
    style = document.createElement('style');
    style.id = styleId;
    style.textContent = `
      .bpx-player-top-left-logo,
      .bpx-player-top-left-title-text,
      .bpx-player-follow {
        display: none !important;
      }

      .bpx-player-control-wrap {
        height: 58px !important;
        opacity: .62 !important;
        transition: opacity 120ms ease !important;
      }
      .bpx-player-control-wrap:hover {
        opacity: .96 !important;
      }
      .bpx-player-control-mask {
        height: 58px !important;
        opacity: .46 !important;
      }
      .bpx-player-control-bottom {
        bottom: 1px !important;
        height: 30px !important;
        min-height: 30px !important;
        padding-inline: 6px !important;
      }
      .bpx-player-ctrl-btn {
        width: 30px !important;
        min-width: 30px !important;
        height: 30px !important;
      }
      .bpx-player-ctrl-btn svg,
      .bpx-player-ctrl-btn img {
        width: 18px !important;
        height: 18px !important;
      }
      .bpx-player-ctrl-time-label {
        height: 30px !important;
        line-height: 30px !important;
        padding-inline: 3px !important;
        font-size: 11px !important;
      }
      .bpx-player-dm-switch {
        width: 30px !important;
        height: 30px !important;
        transform: scale(.72) !important;
        transform-origin: center !important;
      }
      .bpx-player-progress-wrap,
      .bpx-player-shadow-progress-area {
        height: 3px !important;
        min-height: 3px !important;
      }
      .bpx-player-progress-wrap {
        bottom: 32px !important;
      }
    `;
    (document.head || document.documentElement).appendChild(style);
  }

  const hidePromotions = (root = document) => {
    const candidates = [];
    if (root instanceof Element && root.matches('a, button, div, span')) {
      candidates.push(root);
    }
    if (root.querySelectorAll) {
      candidates.push(...root.querySelectorAll('a, button, div, span'));
    }
    for (const element of candidates) {
      const text = (element.textContent || '').replace(/\s+/g, '');
      if (text.length > 36 ||
          (!text.includes('进入哔哩哔哩') && !text.includes('观看更高清'))) {
        continue;
      }
      const overlay = element.closest(
        'a, button, .bpx-player-toast-wrap, .bpx-player-dialog-wrap, .bpx-player-video-toast'
      ) || element;
      overlay.style.setProperty('display', 'none', 'important');
    }
  };

  const applyPreferredRate = (root = document) => {
    const rate = Number(window.__genshinGuidePlaybackRate);
    if (!Number.isFinite(rate) || rate <= 0) return;
    const videos = [];
    if (root instanceof HTMLVideoElement) videos.push(root);
    if (root.querySelectorAll) videos.push(...root.querySelectorAll('video'));
    for (const video of videos) {
      video.defaultPlaybackRate = rate;
      video.playbackRate = rate;
    }
  };

  hidePromotions();
  applyPreferredRate();
  if (window.__genshinGuideCompactObserver) {
    window.__genshinGuideCompactObserver.disconnect();
  }
  window.__genshinGuideCompactObserver = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      if (mutation.type === 'characterData') {
        hidePromotions(mutation.target.parentElement || document);
      }
      for (const node of mutation.addedNodes) {
        if (node instanceof Element) {
          hidePromotions(node);
          applyPreferredRate(node);
        }
      }
    }
  });
  window.__genshinGuideCompactObserver.observe(document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true
  });
  return true;
})()
"#;

const SET_PLAYBACK_RATE_SCRIPT: &str = r#"
(() => {
  const requested = __RATE__;
  window.__genshinGuidePlaybackRate = requested;
  const videos = Array.from(document.querySelectorAll('video'))
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return (b.width * b.height) - (a.width * a.height);
    });
  const video = videos[0];
  if (!video) {
    return { ok: true, pending: true, rate: requested };
  }
  video.defaultPlaybackRate = requested;
  video.playbackRate = requested;
  return { ok: true, pending: false, rate: video.playbackRate };
})()
"#;

const SHOW_QUALITY_SCRIPT: &str = r#"
(() => {
  const isVisible = (element) => {
    if (!element) return false;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 8 && rect.height > 8 &&
      style.display !== 'none' && style.visibility !== 'hidden';
  };
  const selectors = [
    '.bpx-player-ctrl-quality',
    '.bilibili-player-video-btn-quality',
    '[class*="player-ctrl-quality"]'
  ];
  const qualityButton = selectors
    .map((selector) => document.querySelector(selector))
    .find(isVisible);
  const video = Array.from(document.querySelectorAll('video'))
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return (b.width * b.height) - (a.width * a.height);
    })[0];
  if (qualityButton) {
    qualityButton.click();
    return {
      ok: true,
      menuOpened: true,
      label: (qualityButton.textContent || '').trim(),
      videoHeight: video ? (video.videoHeight || 0) : 0
    };
  }
  if (!video) return { ok: false, reason: 'no-video' };
  return {
    ok: true,
    menuOpened: false,
    label: '',
    videoHeight: video.videoHeight || 0
  };
})()
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityState {
    ok: bool,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeedState {
    ok: bool,
    rate: Option<f64>,
    message: String,
}

pub fn apply_compact_layout(webview: &tauri::Webview) -> Result<(), String> {
    webview
        .eval(COMPACT_PLAYER_SCRIPT)
        .map_err(|error| error.to_string())
}

pub fn show_quality(app: &AppHandle) -> Result<(), String> {
    let content = app
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;
    let app_for_result = app.clone();
    content
        .eval_with_callback(SHOW_QUALITY_SCRIPT, move |raw| {
            emit_quality_result(&app_for_result, &raw);
        })
        .map_err(|error| error.to_string())
}

pub fn apply_playback_rate(webview: &tauri::Webview, rate: f64) -> Result<(), String> {
    webview
        .eval(playback_rate_script(rate))
        .map_err(|error| error.to_string())
}

pub fn set_playback_rate(app: &AppHandle, rate: f64) -> Result<(), String> {
    let content = app
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;
    let app_for_result = app.clone();
    content
        .eval_with_callback(playback_rate_script(rate), move |raw| {
            emit_speed_result(&app_for_result, &raw);
        })
        .map_err(|error| error.to_string())
}

fn playback_rate_script(rate: f64) -> String {
    SET_PLAYBACK_RATE_SCRIPT.replace("__RATE__", &rate.to_string())
}

fn emit_quality_result(app: &AppHandle, raw: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            emit_quality_state(app, false, "无法读取当前画质");
            return;
        }
    };
    if !parsed
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        emit_quality_state(app, false, "播放器尚未就绪");
        return;
    }
    let height = parsed
        .get("videoHeight")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let menu_opened = parsed
        .get("menuOpened")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if menu_opened {
        emit_quality_state(app, true, "已打开 B 站画质菜单");
    } else if height > 0 {
        emit_quality_state(
            app,
            true,
            &format!("当前视频源约 {height}P；已默认请求可用最高画质，B 站嵌入页未提供切换菜单"),
        );
    } else {
        emit_quality_state(
            app,
            true,
            "已默认请求可用最高画质；B 站嵌入页未提供切换菜单",
        );
    }
}

fn emit_quality_state(app: &AppHandle, ok: bool, message: &str) {
    let _ = app.emit_to(
        "controls",
        "quality-state",
        QualityState {
            ok,
            message: message.to_string(),
        },
    );
}

fn emit_speed_result(app: &AppHandle, raw: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            emit_speed_state(app, false, None, "无法确认当前播放速度");
            return;
        }
    };
    if !parsed
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        emit_speed_state(app, false, None, "当前页面未找到可调速的视频");
        return;
    }
    let rate = parsed.get("rate").and_then(serde_json::Value::as_f64);
    let pending = parsed
        .get("pending")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = match rate {
        Some(rate) if pending => format!("已设为 {rate}×，播放器就绪后生效"),
        Some(rate) => format!("播放速度 {rate}×"),
        None => "无法确认当前播放速度".to_string(),
    };
    emit_speed_state(app, rate.is_some(), rate, &message);
}

fn emit_speed_state(app: &AppHandle, ok: bool, rate: Option<f64>, message: &str) {
    let _ = app.emit_to(
        "controls",
        "speed-state",
        SpeedState {
            ok,
            rate,
            message: message.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_layout_hides_large_metadata_but_keeps_compact_controls() {
        assert!(COMPACT_PLAYER_SCRIPT.contains("bpx-player-top-left-title-text"));
        assert!(COMPACT_PLAYER_SCRIPT.contains("bpx-player-top-left-logo"));
        assert!(COMPACT_PLAYER_SCRIPT.contains("bpx-player-follow"));
        assert!(COMPACT_PLAYER_SCRIPT.contains("bpx-player-progress-wrap"));
        assert!(COMPACT_PLAYER_SCRIPT.contains("MutationObserver"));
    }

    #[test]
    fn quality_probe_uses_native_menu_or_reports_real_video_height() {
        assert!(SHOW_QUALITY_SCRIPT.contains("bpx-player-ctrl-quality"));
        assert!(SHOW_QUALITY_SCRIPT.contains("video.videoHeight"));
        assert!(SHOW_QUALITY_SCRIPT.contains("qualityButton.click()"));
    }

    #[test]
    fn playback_rate_script_sets_and_reads_the_real_video_rate() {
        let script = playback_rate_script(1.5);
        assert!(script.contains("const requested = 1.5"));
        assert!(script.contains("video.defaultPlaybackRate = requested"));
        assert!(script.contains("rate: video.playbackRate"));
        assert!(!script.contains("__RATE__"));
        assert!(COMPACT_PLAYER_SCRIPT.contains("__genshinGuidePlaybackRate"));
    }
}
