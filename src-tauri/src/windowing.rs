use crate::{settings::clamp_opacity, OverlayState, CONTROL_HEIGHT};
use tauri::{
    Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Rect,
    Size, Webview, Window,
};

pub fn layout_webviews(window: &Window, locked: bool) -> Result<(), String> {
    let inner = window.inner_size().map_err(|error| error.to_string())?;
    if inner.width == 0 || inner.height <= 1 {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let bar_height = ((CONTROL_HEIGHT * scale).round().max(1.0) as u32).clamp(1, inner.height - 1);

    let controls = window
        .app_handle()
        .get_webview("controls")
        .ok_or_else(|| "控制条未就绪".to_string())?;
    let content = window
        .app_handle()
        .get_webview("content")
        .ok_or_else(|| "攻略视图未就绪".to_string())?;

    if locked {
        controls.hide().map_err(|error| error.to_string())?;
        content
            .set_bounds(Rect {
                position: Position::Physical(PhysicalPosition::new(0, 0)),
                size: Size::Physical(inner),
            })
            .map_err(|error| error.to_string())?;
    } else {
        controls.show().map_err(|error| error.to_string())?;
        controls
            .set_bounds(Rect {
                position: Position::Physical(PhysicalPosition::new(0, 0)),
                size: Size::Physical(PhysicalSize::new(inner.width, bar_height)),
            })
            .map_err(|error| error.to_string())?;
        content
            .set_bounds(Rect {
                position: Position::Physical(PhysicalPosition::new(0, bar_height as i32)),
                size: Size::Physical(PhysicalSize::new(
                    inner.width,
                    inner.height.saturating_sub(bar_height),
                )),
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn set_click_through(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let window = app
        .get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?;
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| error.to_string())?;
    let state = app.state::<OverlayState>();
    state
        .locked
        .store(enabled, std::sync::atomic::Ordering::Release);
    layout_webviews(&window, enabled)?;
    crate::emit_overlay_state(app);
    Ok(())
}

pub fn toggle_visibility(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?;
    if window.is_visible().map_err(|error| error.to_string())? {
        window.hide().map_err(|error| error.to_string())
    } else {
        window.show().map_err(|error| error.to_string())?;
        window
            .set_always_on_top(true)
            .map_err(|error| error.to_string())
    }
}

pub fn apply_initial_opacity(app: &tauri::AppHandle, value: u8) -> Result<(), String> {
    let value = clamp_opacity(value);
    let window = app
        .get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?;
    apply_native_opacity(&window, value)
}

pub fn set_opacity(app: &tauri::AppHandle, value: u8) -> Result<u8, String> {
    let value = clamp_opacity(value);
    let window = app
        .get_window("overlay")
        .ok_or_else(|| "悬浮窗未就绪".to_string())?;
    let state = app.state::<OverlayState>();
    let previous = state.settings.lock().map_err(|_| "设置锁已损坏")?.opacity;
    apply_native_opacity(&window, value)?;
    let save_error = {
        let mut settings = state.settings.lock().map_err(|_| "设置锁已损坏")?;
        settings.opacity = value;
        match settings.save(&state.settings_path) {
            Ok(()) => None,
            Err(error) => {
                settings.opacity = previous;
                Some(error)
            }
        }
    };
    if let Some(error) = save_error {
        let result = match apply_native_opacity(&window, previous) {
            Ok(()) => Err(format!("保存透明度失败：{error}")),
            Err(rollback_error) => Err(format!(
                "保存透明度失败：{error}；恢复原透明度也失败：{rollback_error}"
            )),
        };
        crate::emit_overlay_state(app);
        return result;
    }
    crate::emit_overlay_state(app);
    Ok(value)
}

#[cfg(windows)]
fn apply_native_opacity(window: &Window, value: u8) -> Result<(), String> {
    use windows::Win32::Foundation::{GetLastError, SetLastError, WIN32_ERROR};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetLayeredWindowAttributes, SetWindowLongW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let alpha = ((u16::from(value) * 255) / 100) as u8;
    unsafe {
        SetLastError(WIN32_ERROR(0));
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let read_error = GetLastError();
        if style == 0 && read_error.0 != 0 {
            return Err(format!(
                "GetWindowLongW failed: {}",
                std::io::Error::from_raw_os_error(read_error.0 as i32)
            ));
        }
        if style & WS_EX_LAYERED.0 as i32 == 0 {
            SetLastError(WIN32_ERROR(0));
            let previous = SetWindowLongW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as i32);
            let write_error = GetLastError();
            if previous == 0 && write_error.0 != 0 {
                return Err(format!(
                    "SetWindowLongW failed: {}",
                    std::io::Error::from_raw_os_error(write_error.0 as i32)
                ));
            }
        }
        SetLayeredWindowAttributes(hwnd, Default::default(), alpha, LWA_ALPHA)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_native_opacity(_window: &Window, _value: u8) -> Result<(), String> {
    Err("当前版本仅支持 Windows".to_string())
}

pub fn place_window(window: &Window, x: Option<f64>, y: Option<f64>) -> Result<(), String> {
    if let (Some(x), Some(y)) = (x, y) {
        window
            .set_position(LogicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
        return clamp_window_to_monitor(window);
    }

    if let Some(monitor) = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
    {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let position: LogicalPosition<f64> = area.position.to_logical(scale);
        let size: LogicalSize<f64> = area.size.to_logical(scale);
        let window_size: LogicalSize<f64> = window
            .inner_size()
            .map_err(|error| error.to_string())?
            .to_logical(scale);
        window
            .set_position(LogicalPosition::new(
                position.x + 16.0,
                position.y + size.height - window_size.height - 16.0,
            ))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn clamp_window_to_monitor(window: &Window) -> Result<(), String> {
    let Some(monitor) = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or(window
            .primary_monitor()
            .map_err(|error| error.to_string())?)
    else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let area_pos: LogicalPosition<f64> = area.position.to_logical(scale);
    let area_size: LogicalSize<f64> = area.size.to_logical(scale);
    let window_size: LogicalSize<f64> = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical(scale);
    let current: LogicalPosition<f64> = window
        .outer_position()
        .map_err(|error| error.to_string())?
        .to_logical(scale);
    let max_x = area_pos.x + (area_size.width - 80.0).max(0.0);
    let max_y = area_pos.y + (area_size.height - 44.0).max(0.0);
    let min_x = area_pos.x - (window_size.width - 80.0).max(0.0);
    let min_y = area_pos.y;
    window
        .set_position(LogicalPosition::new(
            current.x.clamp(min_x, max_x),
            current.y.clamp(min_y, max_y),
        ))
        .map_err(|error| error.to_string())
}

pub fn initial_webview_bounds(
    width: f64,
    height: f64,
) -> (
    LogicalPosition<f64>,
    LogicalSize<f64>,
    LogicalPosition<f64>,
    LogicalSize<f64>,
) {
    (
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(width, CONTROL_HEIGHT),
        LogicalPosition::new(0.0, CONTROL_HEIGHT),
        LogicalSize::new(width, (height - CONTROL_HEIGHT).max(1.0)),
    )
}

pub fn emit_navigation_state(webview: &Webview, url: String, finished: bool) {
    let _ = webview.app_handle().emit_to(
        "controls",
        "navigation-state",
        serde_json::json!({ "url": url, "finished": finished }),
    );
}
