use crate::OverlayState;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

pub const TOGGLE_MEDIA_SCRIPT: &str = r#"
(() => {
  function collect(root, output) {
    if (!root || !root.querySelectorAll) return;
    for (const video of root.querySelectorAll('video')) output.push(video);
    for (const element of root.querySelectorAll('*')) {
      if (element.shadowRoot) collect(element.shadowRoot, output);
    }
    for (const frame of root.querySelectorAll('iframe')) {
      try { collect(frame.contentDocument, output); } catch (_) {}
    }
  }
  const videos = [];
  collect(document, videos);
  const visible = videos.filter((video) => {
    const rect = video.getBoundingClientRect();
    const style = getComputedStyle(video);
    return rect.width > 24 && rect.height > 24 &&
      style.display !== 'none' && style.visibility !== 'hidden';
  });
  const candidates = visible.length ? visible : videos;
  candidates.sort((left, right) => {
    const a = left.getBoundingClientRect();
    const b = right.getBoundingClientRect();
    return (b.width * b.height) - (a.width * a.height);
  });
  const video = candidates[0];
  if (!video) return { ok: false, reason: 'no-video' };
  window.__guideOverlayPlayError = '';
  if (video.paused || video.ended) {
    try {
      const result = video.play();
      if (result && result.catch) {
        result.catch((error) => {
          window.__guideOverlayPlayError = String(error || 'play-failed');
        });
      }
      return { ok: true, requested: 'play' };
    } catch (error) {
      return { ok: false, reason: String(error || 'play-failed') };
    }
  }
  video.pause();
  return { ok: true, requested: 'pause' };
})()
"#;

pub const READ_MEDIA_STATE_SCRIPT: &str = r#"
(() => {
  function collect(root, output) {
    if (!root || !root.querySelectorAll) return;
    for (const video of root.querySelectorAll('video')) output.push(video);
    for (const element of root.querySelectorAll('*')) {
      if (element.shadowRoot) collect(element.shadowRoot, output);
    }
    for (const frame of root.querySelectorAll('iframe')) {
      try { collect(frame.contentDocument, output); } catch (_) {}
    }
  }
  const videos = [];
  collect(document, videos);
  videos.sort((left, right) => {
    const a = left.getBoundingClientRect();
    const b = right.getBoundingClientRect();
    return (b.width * b.height) - (a.width * a.height);
  });
  const video = videos[0];
  if (!video) return { ok: false, reason: 'no-video' };
  const error = window.__guideOverlayPlayError || '';
  window.__guideOverlayPlayError = '';
  return {
    ok: !error,
    reason: error,
    paused: video.paused,
    ended: video.ended,
    currentTime: video.currentTime || 0
  };
})()
"#;

const SEEK_MEDIA_SCRIPT_TEMPLATE: &str = r#"
(() => {
  function collect(root, output) {
    if (!root || !root.querySelectorAll) return;
    for (const video of root.querySelectorAll('video')) output.push(video);
    for (const element of root.querySelectorAll('*')) {
      if (element.shadowRoot) collect(element.shadowRoot, output);
    }
    for (const frame of root.querySelectorAll('iframe')) {
      try { collect(frame.contentDocument, output); } catch (_) {}
    }
  }
  const videos = [];
  collect(document, videos);
  const visible = videos.filter((video) => {
    const rect = video.getBoundingClientRect();
    const style = getComputedStyle(video);
    return rect.width > 24 && rect.height > 24 &&
      style.display !== 'none' && style.visibility !== 'hidden';
  });
  const candidates = visible.length ? visible : videos;
  candidates.sort((left, right) => {
    const a = left.getBoundingClientRect();
    const b = right.getBoundingClientRect();
    return (b.width * b.height) - (a.width * a.height);
  });
  const video = candidates[0];
  if (!video) return { ok: false, reason: 'no-video' };
  const before = Number.isFinite(video.currentTime) ? video.currentTime : 0;
  const duration = Number.isFinite(video.duration) ? video.duration : null;
  const delta = __SEEK_DELTA__;
  let target = Math.max(0, before + delta);
  if (duration !== null) target = Math.min(duration, target);
  try {
    video.currentTime = target;
  } catch (error) {
    return { ok: false, reason: String(error || 'seek-failed') };
  }
  return {
    ok: true,
    before,
    currentTime: Number.isFinite(video.currentTime) ? video.currentTime : target,
    duration
  };
})()
"#;

pub const SEEK_SECONDS: f64 = 10.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub ok: bool,
    pub paused: Option<bool>,
    pub message: String,
}

pub fn toggle_media(app: &AppHandle) -> Result<(), String> {
    let request_id = app
        .state::<OverlayState>()
        .media_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    execute_script(app, TOGGLE_MEDIA_SCRIPT, ScriptStage::Toggle, request_id)
}

pub fn seek_backward(app: &AppHandle) -> Result<(), String> {
    seek_media(app, -SEEK_SECONDS)
}

pub fn seek_forward(app: &AppHandle) -> Result<(), String> {
    seek_media(app, SEEK_SECONDS)
}

fn seek_media(app: &AppHandle, delta: f64) -> Result<(), String> {
    let request_id = app
        .state::<OverlayState>()
        .media_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let script = SEEK_MEDIA_SCRIPT_TEMPLATE.replace("__SEEK_DELTA__", &delta.to_string());
    let content = app
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;
    let app_for_result = app.clone();
    content
        .eval_with_callback(&script, move |raw| {
            if is_current_request(&app_for_result, request_id) {
                handle_seek_result(&app_for_result, delta, &raw);
            }
        })
        .map_err(|error| error.to_string())
}

pub fn invalidate_pending(app: &AppHandle) {
    app.state::<OverlayState>()
        .media_generation
        .fetch_add(1, Ordering::AcqRel);
}

#[derive(Clone, Copy)]
enum ScriptStage {
    Toggle,
    Read,
}

fn execute_script(
    app: &AppHandle,
    script: &str,
    stage: ScriptStage,
    request_id: u64,
) -> Result<(), String> {
    let content = app
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;
    let app_for_result = app.clone();
    content
        .eval_with_callback(script, move |raw| {
            if is_current_request(&app_for_result, request_id) {
                handle_script_result(&app_for_result, stage, request_id, &raw);
            }
        })
        .map_err(|error| error.to_string())
}

fn is_current_request(app: &AppHandle, request_id: u64) -> bool {
    app.state::<OverlayState>()
        .media_generation
        .load(Ordering::Acquire)
        == request_id
}

fn handle_script_result(app: &AppHandle, stage: ScriptStage, request_id: u64, raw: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            emit_media_state(
                app,
                MediaState {
                    ok: false,
                    paused: None,
                    message: "无法确认视频播放状态".to_string(),
                },
            );
            return;
        }
    };
    if !parsed
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        let no_video = parsed.get("reason").and_then(|value| value.as_str()) == Some("no-video");
        emit_media_state(
            app,
            MediaState {
                ok: false,
                paused: None,
                message: if no_video {
                    "当前页面未找到可控制的视频".to_string()
                } else {
                    "视频播放被网页拒绝，请先手动播放一次".to_string()
                },
            },
        );
        return;
    }

    match stage {
        ScriptStage::Toggle => schedule_state_read(app, request_id),
        ScriptStage::Read => {
            let paused = parsed
                .get("paused")
                .and_then(|value| value.as_bool())
                .unwrap_or(true);
            emit_media_state(
                app,
                MediaState {
                    ok: true,
                    paused: Some(paused),
                    message: if paused {
                        "Ⅱ 攻略已暂停".to_string()
                    } else {
                        "▶ 攻略继续".to_string()
                    },
                },
            );
        }
    }
}

fn handle_seek_result(app: &AppHandle, delta: f64, raw: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            emit_media_state(
                app,
                MediaState {
                    ok: false,
                    paused: None,
                    message: "无法确认视频跳转结果".to_string(),
                },
            );
            return;
        }
    };
    if !parsed
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        let no_video = parsed.get("reason").and_then(|value| value.as_str()) == Some("no-video");
        emit_media_state(
            app,
            MediaState {
                ok: false,
                paused: None,
                message: if no_video {
                    "当前页面未找到可控制的视频".to_string()
                } else {
                    "当前视频暂时不能跳转".to_string()
                },
            },
        );
        return;
    }

    let before = parsed
        .get("before")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let current = parsed
        .get("currentTime")
        .and_then(|value| value.as_f64())
        .unwrap_or(before);
    let moved = (current - before).abs();
    let message = if moved < 0.25 {
        if delta < 0.0 {
            "已到视频开头".to_string()
        } else {
            "已到视频结尾".to_string()
        }
    } else if delta < 0.0 {
        format!("↶ 已快退 {:.0} 秒", moved)
    } else {
        format!("↷ 已快进 {:.0} 秒", moved)
    };
    emit_media_state(
        app,
        MediaState {
            ok: true,
            paused: None,
            message,
        },
    );
}

fn schedule_state_read(app: &AppHandle, request_id: u64) {
    let dispatcher = app.clone();
    let app_for_read = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(350));
        let _ = dispatcher.run_on_main_thread(move || {
            if is_current_request(&app_for_read, request_id) {
                let _ = execute_script(
                    &app_for_read,
                    READ_MEDIA_STATE_SCRIPT,
                    ScriptStage::Read,
                    request_id,
                );
            }
        });
    });
}

fn emit_media_state(app: &AppHandle, state: MediaState) {
    let _ = app.emit_to("controls", "media-state", state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_use_largest_video_and_verify_real_state() {
        assert!(TOGGLE_MEDIA_SCRIPT.contains("querySelectorAll('video')"));
        assert!(TOGGLE_MEDIA_SCRIPT.contains("video.pause()"));
        assert!(TOGGLE_MEDIA_SCRIPT.contains("video.play()"));
        assert!(READ_MEDIA_STATE_SCRIPT.contains("paused: video.paused"));
        assert!(READ_MEDIA_STATE_SCRIPT.contains("no-video"));
        assert!(SEEK_MEDIA_SCRIPT_TEMPLATE.contains("video.currentTime = target"));
        assert!(SEEK_MEDIA_SCRIPT_TEMPLATE.contains("Math.max(0"));
        assert!(SEEK_MEDIA_SCRIPT_TEMPLATE.contains("Math.min(duration"));
        assert!(seek_script(-SEEK_SECONDS).contains("const delta = -10"));
    }

    fn seek_script(delta: f64) -> String {
        SEEK_MEDIA_SCRIPT_TEMPLATE.replace("__SEEK_DELTA__", &delta.to_string())
    }
}
