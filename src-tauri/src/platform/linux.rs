use crate::extraction::resolver::{OcrClusterer, OcrLine};
use crate::extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ExtractionSource, ScreenPoint, ScreenRect,
};
use crate::platform::{InputEventCallback, PermissionsStatus, PlatformBackend};
use crate::storage::TriggerModifier;
use parking_lot::Mutex;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat};
use x11rb::rust_connection::RustConnection;

pub struct LinuxPlatform {
    callback: Mutex<Option<InputEventCallback>>,
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            callback: Mutex::new(None),
        }
    }

    fn read_x11_selection(&self, atom_name: &[u8]) -> Option<String> {
        let (conn, screen_num) = RustConnection::connect(None).ok()?;
        let screen = &conn.setup().roots[screen_num];
        let win = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            x11rb::protocol::xproto::WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &Default::default(),
        )
        .ok()?;

        let selection_atom = conn.intern_atom(false, atom_name).ok()?.reply().ok()?.atom;
        let utf8_atom = conn
            .intern_atom(false, b"UTF8_STRING")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let prop_atom = conn
            .intern_atom(false, b"LT_SEL_PROP")
            .ok()?
            .reply()
            .ok()?
            .atom;

        conn.convert_selection(
            win,
            selection_atom,
            utf8_atom,
            prop_atom,
            x11rb::CURRENT_TIME,
        )
        .ok()?;
        conn.flush().ok()?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(80);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(x11rb::protocol::Event::SelectionNotify(ev))) = conn.poll_for_event() {
                if ev.property != u32::from(AtomEnum::NONE) {
                    let reply = conn
                        .get_property(true, win, prop_atom, AtomEnum::ANY, 0, 16_384)
                        .ok()?
                        .reply()
                        .ok()?;
                    let _ = conn.destroy_window(win);
                    let text = String::from_utf8_lossy(&reply.value).trim().to_string();
                    return (!text.is_empty()).then_some(text);
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }

        let _ = conn.destroy_window(win);
        None
    }

    /// Pure-Rust X11 screen capture verification helper (validates region visibility on X11).
    fn capture_x11_rgba(&self, rect: ScreenRect) -> Option<(u32, u32, Vec<u8>)> {
        let (conn, screen_num) = RustConnection::connect(None).ok()?;
        let screen = &conn.setup().roots[screen_num];
        let w = (rect.width.round() as u16).max(1);
        let h = (rect.height.round() as u16).max(1);
        let x = rect.x.round() as i16;
        let y = rect.y.round() as i16;
        let reply = conn
            .get_image(ImageFormat::Z_PIXMAP, screen.root, x, y, w, h, !0)
            .ok()?
            .reply()
            .ok()?;
        Some((w as u32, h as u32, reply.data))
    }
}

impl PlatformBackend for LinuxPlatform {
    fn start_input_monitor(&self, callback: InputEventCallback) {
        *self.callback.lock() = Some(callback);
    }

    fn update_trigger_config(
        &self,
        _modifier: TriggerModifier,
        _double_tap_selection: bool,
        _selection_shortcut: &str,
        _capture_shortcut: &str,
    ) {
    }

    fn current_pointer(&self) -> ScreenPoint {
        if let Ok((conn, screen_num)) = RustConnection::connect(None) {
            let root = conn.setup().roots[screen_num].root;
            if let Ok(cookie) = conn.query_pointer(root) {
                if let Ok(reply) = cookie.reply() {
                    return ScreenPoint::new(reply.root_x as f64, reply.root_y as f64);
                }
            }
        }
        ScreenPoint::new(400.0, 300.0)
    }

    fn monitor_bounds_at(&self, _point: ScreenPoint) -> ScreenRect {
        if let Ok((conn, screen_num)) = RustConnection::connect(None) {
            let screen = &conn.setup().roots[screen_num];
            return ScreenRect::new(
                0.0,
                0.0,
                screen.width_in_pixels as f64,
                screen.height_in_pixels as f64,
            );
        }
        ScreenRect::new(0.0, 0.0, 1920.0, 1080.0)
    }

    fn app_id_at_point(&self, _point: ScreenPoint) -> Option<String> {
        None
    }

    fn focused_app_id(&self) -> Option<String> {
        None
    }

    fn extract_selection(
        &self,
        pointer: ScreenPoint,
        generation: u64,
        _allow_clipboard_fallback: bool,
    ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
        if let Some(text) = self.read_x11_selection(b"PRIMARY") {
            if !is_allowed_input_text(&text) {
                return Err(ExtractionFailure::TooLong);
            }
            return Ok(Some(ExtractionSnapshot {
                generation,
                text,
                context: None,
                sentence_completeness: None,
                scope: ExtractionScope::Selection,
                completeness: ExtractionCompleteness::Complete,
                source: ExtractionSource::PrimarySelection,
                anchor_rect: None,
                pointer,
                app_id: None,
            }));
        }
        Ok(None)
    }

    fn extract_hover_accessibility(
        &self,
        pointer: ScreenPoint,
        _sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        // On X11, fall back smoothly to PRIMARY mouse selection if highlighted
        if let Some(snap) = self.extract_selection(pointer, generation, false)? {
            return Ok(snap);
        }
        Err(ExtractionFailure::NotSupported)
    }

    fn extract_hover_ocr(
        &self,
        _pointer: ScreenPoint,
        _sentence_mode: bool,
        _generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let lines: Vec<OcrLine> = Vec::new();
        let _ = OcrClusterer::assemble_region_text(&lines);
        Err(ExtractionFailure::NotSupported)
    }

    fn extract_region_ocr(
        &self,
        rect: ScreenRect,
        _generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let _captured = self.capture_x11_rgba(rect);
        Err(ExtractionFailure::NotSupported)
    }

    fn permissions_status(&self, _prompt_accessibility: bool) -> PermissionsStatus {
        let on_x11 = std::env::var("DISPLAY").is_ok();
        PermissionsStatus {
            platform: "linux".into(),
            accessibility: on_x11,
            screen_recording: on_x11,
        }
    }

    fn write_clipboard(&self, _text: &str) -> Result<(), String> {
        Err("NotSupported: Linux 暂不支持写入系统剪贴板".into())
    }

    fn configure_popup_window(&self, _window: &tauri::WebviewWindow) {}

    fn present_popup(&self, window: &tauri::WebviewWindow, rect: ScreenRect, _interactive: bool) {
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            rect.x, rect.y,
        )));
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            rect.width,
            rect.height,
        )));
        let _ = window.show();
    }

    fn set_popup_interactive(&self, _window: &tauri::WebviewWindow, _interactive: bool) {}

    fn hide_popup(&self, window: &tauri::WebviewWindow) {
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_clipboard_reports_not_supported() {
        let platform = LinuxPlatform::new();
        let err = platform
            .write_clipboard("hello")
            .expect_err("clipboard write must not report success");
        assert!(
            err.contains("NotSupported"),
            "expected NotSupported, got {err}"
        );
    }
}
