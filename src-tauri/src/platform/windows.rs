use crate::extraction::resolver::{
    resolve_word_in_utf16, OcrClusterer, OcrLine, OcrWord, SentenceResolver, SentenceWindow,
    Utf16Range,
};
use crate::extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ExtractionSource, ScreenPoint, ScreenRect,
};
use crate::overlay::CapturedImage;
use crate::platform::{
    InputEventCallback, PermissionsStatus, PlatformBackend, PlatformInputEvent, ShortcutSpec,
    SHORTCUT_MOD_ALT, SHORTCUT_MOD_CTRL, SHORTCUT_MOD_META, SHORTCUT_MOD_SHIFT,
};
use crate::storage::TriggerModifier;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use windows::core::{HSTRING, PWSTR};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, TextUnit_Paragraph, TextUnit_Word,
    UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_C, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RWIN, VK_S, VK_SHIFT, VK_T,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetWindowLongPtrW, GetWindowThreadProcessId, SetWindowLongPtrW, SetWindowPos,
    SetWindowsHookExW, ShowWindow, TranslateMessage, WindowFromPoint, GWL_EXSTYLE, HWND_TOPMOST,
    KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE,
    WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

const CF_UNICODETEXT_ID: u32 = 13;

struct HookRuntimeState {
    trigger_down: bool,
    sentence_mod_down: bool,
    other_key_during_tap: bool,
    tap_press_time: Option<Instant>,
    tap_press_point: ScreenPoint,
    last_tap_release: Option<Instant>,
    shortcut_key_down: Option<u32>,
}

impl Default for HookRuntimeState {
    fn default() -> Self {
        Self {
            trigger_down: false,
            sentence_mod_down: false,
            other_key_during_tap: false,
            tap_press_time: None,
            tap_press_point: ScreenPoint::default(),
            last_tap_release: None,
            shortcut_key_down: None,
        }
    }
}

static INPUT_CB: LazyLock<Mutex<Option<InputEventCallback>>> = LazyLock::new(|| Mutex::new(None));
static HOOK_STATE: LazyLock<Mutex<HookRuntimeState>> =
    LazyLock::new(|| Mutex::new(HookRuntimeState::default()));
static TRIGGER_KIND: AtomicI32 = AtomicI32::new(0);
static DOUBLE_TAP_ENABLED: AtomicBool = AtomicBool::new(true);
static SHORTCUT_RECORDING: AtomicBool = AtomicBool::new(false);
static SELECTION_SHORTCUT: LazyLock<Mutex<Option<(u32, u32)>>> =
    LazyLock::new(|| Mutex::new(Some((SHORTCUT_MOD_ALT | SHORTCUT_MOD_SHIFT, VK_T.0 as u32))));
static CAPTURE_SHORTCUT: LazyLock<Mutex<Option<(u32, u32)>>> =
    LazyLock::new(|| Mutex::new(Some((SHORTCUT_MOD_ALT | SHORTCUT_MOD_SHIFT, VK_S.0 as u32))));
static HOOK_STARTED: AtomicBool = AtomicBool::new(false);

fn emit_event(ev: PlatformInputEvent) {
    let cb = INPUT_CB.lock().clone();
    if let Some(callback) = cb {
        callback(ev);
    }
}

fn dpi_scale() -> f64 {
    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 {
        1.0
    } else {
        (dpi as f64) / 96.0
    }
}

fn physical_to_logical_pt(px: i32, py: i32) -> ScreenPoint {
    let s = dpi_scale();
    ScreenPoint::new((px as f64) / s, (py as f64) / s)
}

fn logical_to_physical_rect(r: ScreenRect) -> (i32, i32, i32, i32) {
    let s = dpi_scale();
    (
        (r.x * s).round() as i32,
        (r.y * s).round() as i32,
        (r.width * s).round().max(1.0) as i32,
        (r.height * s).round().max(1.0) as i32,
    )
}

fn read_cursor_logical() -> ScreenPoint {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    physical_to_logical_pt(pt.x, pt.y)
}

fn key_down_async(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

fn is_trigger_vk(vk: u32, kind: i32) -> bool {
    match kind {
        1 => vk == VK_LMENU.0 as u32,
        2 => vk == VK_RMENU.0 as u32,
        3 => vk == VK_LCONTROL.0 as u32 || vk == VK_RCONTROL.0 as u32 || vk == VK_CONTROL.0 as u32,
        _ => vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32 || vk == VK_MENU.0 as u32,
    }
}

fn is_sentence_vk(vk: u32, kind: i32) -> bool {
    if kind == 3 {
        vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32 || vk == VK_MENU.0 as u32
    } else {
        vk == VK_LCONTROL.0 as u32 || vk == VK_RCONTROL.0 as u32 || vk == VK_CONTROL.0 as u32
    }
}

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && lparam.0 != 0 && !SHORTCUT_RECORDING.load(Ordering::Relaxed) {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        let vk = kb.vkCode;
        let kind = TRIGGER_KIND.load(Ordering::Relaxed);
        let pointer = read_cursor_logical();
        let win_held = key_down_async(VK_LWIN.0 as i32) || key_down_async(VK_RWIN.0 as i32);
        let mut active_mods = 0u32;
        if key_down_async(VK_CONTROL.0 as i32) {
            active_mods |= SHORTCUT_MOD_CTRL;
        }
        if key_down_async(VK_MENU.0 as i32) {
            active_mods |= SHORTCUT_MOD_ALT;
        }
        if key_down_async(VK_SHIFT.0 as i32) {
            active_mods |= SHORTCUT_MOD_SHIFT;
        }
        if win_held {
            active_mods |= SHORTCUT_MOD_META;
        }
        let sel_bind = *SELECTION_SHORTCUT.lock();
        let cap_bind = *CAPTURE_SHORTCUT.lock();

        if is_up {
            let mut st = HOOK_STATE.lock();
            if st.shortcut_key_down == Some(vk) {
                st.shortcut_key_down = None;
            }
        }

        if is_down && vk == VK_ESCAPE.0 as u32 {
            emit_event(PlatformInputEvent::EscapePressed);
        } else if is_down
            && sel_bind.is_some_and(|(mods, target_vk)| active_mods == mods && vk == target_vk)
        {
            let mut st = HOOK_STATE.lock();
            if st.shortcut_key_down != Some(vk) {
                st.shortcut_key_down = Some(vk);
                st.other_key_during_tap = true;
                drop(st);
                emit_event(PlatformInputEvent::ShortcutSelection { pointer });
            }
        } else if is_down
            && cap_bind.is_some_and(|(mods, target_vk)| active_mods == mods && vk == target_vk)
        {
            let mut st = HOOK_STATE.lock();
            if st.shortcut_key_down != Some(vk) {
                st.shortcut_key_down = Some(vk);
                st.other_key_during_tap = true;
                drop(st);
                emit_event(PlatformInputEvent::ShortcutCapture);
            }
        } else if is_trigger_vk(vk, kind) {
            if is_down && !win_held {
                let mut st = HOOK_STATE.lock();
                if !st.trigger_down {
                    let sent_now = if kind == 3 {
                        key_down_async(VK_MENU.0 as i32)
                    } else {
                        key_down_async(VK_CONTROL.0 as i32)
                    };
                    st.trigger_down = true;
                    st.sentence_mod_down = sent_now;
                    st.other_key_during_tap = sent_now;
                    st.tap_press_time = Some(Instant::now());
                    st.tap_press_point = pointer;
                    drop(st);
                    emit_event(PlatformInputEvent::TriggerPressed {
                        sentence_modifier: sent_now,
                        pointer,
                    });
                }
            } else if is_up {
                let mut st = HOOK_STATE.lock();
                if st.trigger_down {
                    st.trigger_down = false;
                    st.sentence_mod_down = false;
                    let now = Instant::now();
                    let clean_tap = !st.other_key_during_tap
                        && st
                            .tap_press_time
                            .is_some_and(|t| now.duration_since(t) <= Duration::from_millis(280))
                        && st.tap_press_point.distance_to(pointer) < 12.0;
                    let valid_tap = DOUBLE_TAP_ENABLED.load(Ordering::Relaxed) && clean_tap;

                    let fire_double_tap = if valid_tap {
                        if st
                            .last_tap_release
                            .is_some_and(|prev| now.duration_since(prev) <= Duration::from_millis(360))
                        {
                            st.last_tap_release = None;
                            true
                        } else {
                            st.last_tap_release = Some(now);
                            false
                        }
                    } else {
                        st.last_tap_release = None;
                        false
                    };
                    drop(st);

                    emit_event(PlatformInputEvent::TriggerReleased {
                        pointer,
                        clean_tap,
                    });
                    if fire_double_tap {
                        emit_event(PlatformInputEvent::DoubleTapSelection { pointer });
                    }
                }
            }
        } else if is_sentence_vk(vk, kind) {
            let mut st = HOOK_STATE.lock();
            st.other_key_during_tap = true;
            if st.trigger_down && st.sentence_mod_down != is_down {
                st.sentence_mod_down = is_down;
                drop(st);
                emit_event(PlatformInputEvent::SentenceModifierChanged {
                    active: is_down,
                    pointer,
                });
            }
        } else if is_down {
            HOOK_STATE.lock().other_key_during_tap = true;
            emit_event(PlatformInputEvent::OtherKeyPressed);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        let ms = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let pointer = physical_to_logical_pt(ms.pt.x, ms.pt.y);
        match wparam.0 as u32 {
            WM_MOUSEMOVE => {
                emit_event(PlatformInputEvent::PointerMoved { pointer });
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                HOOK_STATE.lock().other_key_during_tap = true;
                emit_event(PlatformInputEvent::GlobalMouseDown { pointer });
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn exe_name_for_hwnd(hwnd: HWND) -> Option<String> {
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 512];
    let mut size = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(handle) };
    if !ok || size == 0 {
        return None;
    }
    let full = String::from_utf16_lossy(&buf[..size as usize]);
    Path::new(&full)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
}

// ============================================================================
// UIAutomation Helpers (Selection, 4-Level Subtree, & Cross-Node Paragraphs)
// ============================================================================

fn create_uia() -> Option<IUIAutomation> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    }
}

fn uia_is_password(elem: &IUIAutomationElement) -> bool {
    unsafe { elem.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) }
}

fn uia_element_rect_logical(elem: &IUIAutomationElement) -> Option<ScreenRect> {
    let r: RECT = unsafe { elem.CurrentBoundingRectangle() }.ok()?;
    let w = (r.right - r.left) as f64;
    let h = (r.bottom - r.top) as f64;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let s = dpi_scale();
    Some(ScreenRect::new(
        (r.left as f64) / s,
        (r.top as f64) / s,
        w / s,
        h / s,
    ))
}

fn first_bounding_rect_logical(sa: *mut SAFEARRAY) -> Option<ScreenRect> {
    if sa.is_null() {
        return None;
    }
    unsafe {
        let lb = SafeArrayGetLBound(sa, 1).ok()?;
        let ub = SafeArrayGetUBound(sa, 1).ok()?;
        if ub - lb + 1 < 4 {
            let _ = SafeArrayDestroy(sa);
            return None;
        }
        let mut vals = [0.0f64; 4];
        for (idx, slot) in vals.iter_mut().enumerate() {
            let i = lb + idx as i32;
            let _ = SafeArrayGetElement(sa, &i, slot as *mut f64 as *mut std::ffi::c_void);
        }
        let _ = SafeArrayDestroy(sa);
        let s = dpi_scale();
        if vals[2] > 0.0 && vals[3] > 0.0 {
            Some(ScreenRect::new(
                vals[0] / s,
                vals[1] / s,
                vals[2] / s,
                vals[3] / s,
            ))
        } else {
            None
        }
    }
}

/// Bounded 4-level, 24-node ControlView subtree hit-test by bounding rectangle.
fn find_deepest_uia_child(
    uia: &IUIAutomation,
    root: &IUIAutomationElement,
    pt_phys: POINT,
    depth: usize,
    budget: &mut usize,
) -> Option<IUIAutomationElement> {
    if depth > 4 || *budget == 0 {
        return None;
    }
    let walker = unsafe { uia.ControlViewWalker() }.ok()?;
    let mut child_opt = unsafe { walker.GetFirstChildElement(root) }.ok();

    while let Some(child) = child_opt {
        if *budget == 0 {
            break;
        }
        *budget -= 1;

        if !uia_is_password(&child) {
            if let Ok(r) = unsafe { child.CurrentBoundingRectangle() } {
                let inside = pt_phys.x >= r.left - 3
                    && pt_phys.x <= r.right + 3
                    && pt_phys.y >= r.top - 3
                    && pt_phys.y <= r.bottom + 3;
                if inside {
                    if let Some(deeper) =
                        find_deepest_uia_child(uia, &child, pt_phys, depth + 1, budget)
                    {
                        return Some(deeper);
                    }
                    return Some(child);
                }
            }
        }
        child_opt = unsafe { walker.GetNextSiblingElement(&child) }.ok();
    }
    None
}

// ============================================================================
// Clipboard Backup -> Synthesized Ctrl+C -> 30ms Restore Fallback
// ============================================================================

fn read_clipboard_unicode() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(handle) = GetClipboardData(CF_UNICODETEXT_ID) {
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0)) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 100_000 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                out = Some(String::from_utf16_lossy(slice));
                let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
            }
        }
        let _ = CloseClipboard();
        out
    }
}

fn write_clipboard_unicode(text: &str) -> Result<(), String> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let byte_len = utf16.len() * 2;

    unsafe {
        OpenClipboard(None).map_err(|e| e.to_string())?;
        let _ = EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len).map_err(|e| {
            let _ = CloseClipboard();
            e.to_string()
        })?;
        let dst = GlobalLock(hmem) as *mut u16;
        if !dst.is_null() {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
            let _ = GlobalUnlock(hmem);
            let _ = SetClipboardData(CF_UNICODETEXT_ID, windows::Win32::Foundation::HANDLE(hmem.0));
        }
        let _ = CloseClipboard();
    }
    Ok(())
}

fn clipboard_fallback_copy_windows() -> Option<String> {
    let backup = read_clipboard_unicode();
    let seq_before = unsafe { GetClipboardSequenceNumber() };

    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_C,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_C,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];

    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    let mut copied = None;
    for _ in 0..7 {
        std::thread::sleep(Duration::from_millis(5));
        if unsafe { GetClipboardSequenceNumber() } != seq_before {
            copied = read_clipboard_unicode();
            break;
        }
    }

    // Immediately restore previous clipboard text
    if let Some(prev) = backup {
        let _ = write_clipboard_unicode(&prev);
    }
    copied
}

// ============================================================================
// Local Offline `Windows.Media.Ocr` + GDI Screen Region Capture
// ============================================================================

fn capture_screen_bgra(px: i32, py: i32, pw: i32, ph: i32) -> Option<Vec<u8>> {
    if pw < 4 || ph < 4 {
        return None;
    }
    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bmp = CreateCompatibleBitmap(screen_dc, pw, ph);
        let old = SelectObject(mem_dc, bmp);

        let ok = BitBlt(mem_dc, 0, 0, pw, ph, screen_dc, px, py, SRCCOPY).is_ok();
        let mut pixels = vec![0u8; (pw as usize) * (ph as usize) * 4];

        if ok {
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: pw,
                    biHeight: -ph, // top-down BGRA
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = GetDIBits(
                mem_dc,
                bmp,
                0,
                ph as u32,
                Some(pixels.as_mut_ptr() as *mut std::ffi::c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if lines == 0 {
                pixels.clear();
            }
        } else {
            pixels.clear();
        }

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        (!pixels.is_empty()).then_some(pixels)
    }
}

fn run_windows_ocr_bgra(
    bgra: &[u8],
    pw: i32,
    ph: i32,
    logical_region: ScreenRect,
) -> Result<Vec<OcrLine>, ExtractionFailure> {
    let ibuffer =
        CryptographicBuffer::CreateFromByteArray(bgra).map_err(|_| ExtractionFailure::NoText)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &ibuffer,
        BitmapPixelFormat::Bgra8,
        pw,
        ph,
        BitmapAlphaMode::Ignore,
    )
    .map_err(|_| ExtractionFailure::NoText)?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .or_else(|_| {
            let lang = Language::CreateLanguage(&HSTRING::from("en-US"))?;
            OcrEngine::TryCreateFromLanguage(&lang)
        })
        .map_err(|_| ExtractionFailure::NotSupported)?;

    let ocr_result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|_| ExtractionFailure::NoText)?
        .get()
        .map_err(|_| ExtractionFailure::NoText)?;

    let raw_lines = ocr_result.Lines().map_err(|_| ExtractionFailure::NoText)?;
    let scale = dpi_scale();
    let mut out_lines = Vec::new();

    for i in 0..raw_lines.Size().unwrap_or(0) {
        let Ok(line) = raw_lines.GetAt(i) else {
            continue;
        };
        let line_text = line.Text().map(|s| s.to_string()).unwrap_or_default();
        let Ok(raw_words) = line.Words() else {
            continue;
        };
        let mut words = Vec::new();
        for j in 0..raw_words.Size().unwrap_or(0) {
            let Ok(w) = raw_words.GetAt(j) else {
                continue;
            };
            let w_text = w.Text().map(|s| s.to_string()).unwrap_or_default();
            let Ok(b) = w.BoundingRect() else {
                continue;
            };
            let rect = ScreenRect::new(
                logical_region.x + (b.X as f64) / scale,
                logical_region.y + (b.Y as f64) / scale,
                (b.Width as f64) / scale,
                (b.Height as f64) / scale,
            );
            words.push(OcrWord {
                text: w_text,
                rect,
                confidence: 0.90,
                trailing: String::new(),
            });
        }
        if !words.is_empty() {
            out_lines.push(OcrLine {
                text: line_text,
                words,
            });
        }
    }

    Ok(out_lines)
}

fn run_windows_ocr(logical_region: ScreenRect) -> Result<Vec<OcrLine>, ExtractionFailure> {
    let (px, py, pw, ph) = logical_to_physical_rect(logical_region);
    let bgra = capture_screen_bgra(px, py, pw, ph).ok_or(ExtractionFailure::NoText)?;
    run_windows_ocr_bgra(&bgra, pw, ph, logical_region)
}

// ============================================================================
// `WindowsPlatform` Implementation
// ============================================================================

pub struct WindowsPlatform;

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformBackend for WindowsPlatform {
    fn start_input_monitor(&self, callback: InputEventCallback) {
        *INPUT_CB.lock() = Some(callback);
        if HOOK_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            let _kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0);
            let _ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    fn update_trigger_config(
        &self,
        modifier: TriggerModifier,
        double_tap_selection: bool,
        selection_shortcut: &str,
        capture_shortcut: &str,
    ) {
        let code = match modifier {
            TriggerModifier::AltOrOption => 0,
            TriggerModifier::LeftOption => 1,
            TriggerModifier::RightOption => 2,
            TriggerModifier::Control => 3,
        };
        TRIGGER_KIND.store(code, Ordering::Relaxed);
        DOUBLE_TAP_ENABLED.store(double_tap_selection, Ordering::Relaxed);
        *SELECTION_SHORTCUT.lock() = ShortcutSpec::parse(selection_shortcut)
            .and_then(|s| s.windows_vk().map(|vk| (s.modifier_mask(), vk)));
        *CAPTURE_SHORTCUT.lock() = ShortcutSpec::parse(capture_shortcut)
            .and_then(|s| s.windows_vk().map(|vk| (s.modifier_mask(), vk)));
    }

    fn set_shortcut_recording(&self, active: bool) {
        SHORTCUT_RECORDING.store(active, Ordering::Relaxed);
    }

    fn current_pointer(&self) -> ScreenPoint {
        read_cursor_logical()
    }

    fn monitor_bounds_at(&self, point: ScreenPoint) -> ScreenRect {
        let s = dpi_scale();
        let pt = POINT {
            x: (point.x * s).round() as i32,
            y: (point.y * s).round() as i32,
        };
        unsafe {
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(hmon, &mut info).as_bool() {
                let r = info.rcWork;
                return ScreenRect::new(
                    (r.left as f64) / s,
                    (r.top as f64) / s,
                    ((r.right - r.left) as f64) / s,
                    ((r.bottom - r.top) as f64) / s,
                );
            }
        }
        ScreenRect::new(0.0, 0.0, 1920.0, 1080.0)
    }

    fn app_id_at_point(&self, point: ScreenPoint) -> Option<String> {
        let s = dpi_scale();
        let pt = POINT {
            x: (point.x * s).round() as i32,
            y: (point.y * s).round() as i32,
        };
        let hwnd = unsafe { WindowFromPoint(pt) };
        exe_name_for_hwnd(hwnd)
    }

    fn focused_app_id(&self) -> Option<String> {
        let hwnd = unsafe { GetForegroundWindow() };
        exe_name_for_hwnd(hwnd)
    }

    fn extract_selection(
        &self,
        pointer: ScreenPoint,
        generation: u64,
        allow_clipboard_fallback: bool,
    ) -> Result<Option<ExtractionSnapshot>, ExtractionFailure> {
        if let Some(uia) = create_uia() {
            if let Ok(focused) = unsafe { uia.GetFocusedElement() } {
                if uia_is_password(&focused) {
                    return Err(ExtractionFailure::BlockedSensitive);
                }
                if let Ok(pattern) =
                    unsafe { focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
                {
                    if let Ok(ranges) = unsafe { pattern.GetSelection() } {
                        let len = unsafe { ranges.Length() }.unwrap_or(0);
                        if len > 0 {
                            if let Ok(range) = unsafe { ranges.GetElement(0) } {
                                let raw = unsafe { range.GetText(5005) }
                                    .map(|b| b.to_string())
                                    .unwrap_or_default();
                                let trimmed = raw.trim().to_string();
                                if !trimmed.is_empty() {
                                    if !is_allowed_input_text(&trimmed) {
                                        return Err(ExtractionFailure::TooLong);
                                    }
                                    let rect = unsafe { range.GetBoundingRectangles() }
                                        .ok()
                                        .and_then(first_bounding_rect_logical);
                                    return Ok(Some(ExtractionSnapshot {
                                        generation,
                                        text: trimmed,
                                        context: None,
                                        sentence_completeness: None,
                                        scope: ExtractionScope::Selection,
                                        completeness: ExtractionCompleteness::Complete,
                                        source: ExtractionSource::Accessibility,
                                        anchor_rect: rect,
                                        pointer,
                                        app_id: self.focused_app_id(),
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }

        if allow_clipboard_fallback {
            if let Some(copied) = clipboard_fallback_copy_windows() {
                let trimmed = copied.trim().to_string();
                if !trimmed.is_empty() {
                    if !is_allowed_input_text(&trimmed) {
                        return Err(ExtractionFailure::TooLong);
                    }
                    return Ok(Some(ExtractionSnapshot {
                        generation,
                        text: trimmed,
                        context: None,
                        sentence_completeness: None,
                        scope: ExtractionScope::Selection,
                        completeness: ExtractionCompleteness::Complete,
                        source: ExtractionSource::ClipboardFallback,
                        anchor_rect: None,
                        pointer,
                        app_id: self.focused_app_id(),
                    }));
                }
            }
        }

        Ok(None)
    }

    fn extract_hover_accessibility(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let uia = create_uia().ok_or(ExtractionFailure::NotSupported)?;
        let s = dpi_scale();
        let pt_phys = POINT {
            x: (pointer.x * s).round() as i32,
            y: (pointer.y * s).round() as i32,
        };

        let mut elem =
            unsafe { uia.ElementFromPoint(pt_phys) }.map_err(|_| ExtractionFailure::PositionUnresolved)?;

        let elem_pid = unsafe { elem.CurrentProcessId() }.unwrap_or(0) as u32;
        if elem_pid == unsafe { GetCurrentProcessId() } {
            return Err(ExtractionFailure::PositionUnresolved);
        }
        if uia_is_password(&elem) {
            return Err(ExtractionFailure::BlockedSensitive);
        }

        // 1. Try TextPattern RangeFromPoint on hit element (or deepest 4-level child)
        let mut text_pat =
            unsafe { elem.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }.ok();

        if text_pat.is_none() {
            let mut budget = 24usize;
            if let Some(deep) = find_deepest_uia_child(&uia, &elem, pt_phys, 1, &mut budget) {
                elem = deep;
                text_pat = unsafe {
                    elem.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                }
                .ok();
            }
        }

        if let Some(pattern) = text_pat {
            if let Ok(point_range) = unsafe { pattern.RangeFromPoint(pt_phys) } {
                let word_range = unsafe { point_range.Clone() }.ok();
                let para_range = unsafe { point_range.Clone() }.ok();

                let mut extracted_word = None;
                let mut word_rect = None;
                if let Some(wr) = word_range {
                    let _ = unsafe { wr.ExpandToEnclosingUnit(TextUnit_Word) };
                    let raw_w = unsafe { wr.GetText(128) }
                        .map(|b| b.to_string())
                        .unwrap_or_default();
                    let w_len = raw_w.encode_utf16().count();
                    if w_len > 0 {
                        if let Some((_, clean_w)) =
                            resolve_word_in_utf16(&raw_w, Utf16Range::new(0, w_len), true)
                        {
                            extracted_word = Some(clean_w);
                            word_rect = unsafe { wr.GetBoundingRectangles() }
                                .ok()
                                .and_then(first_bounding_rect_logical);
                        }
                    }
                }

                let mut resolved_sentence = None;
                if let Some(pr) = para_range {
                    let _ = unsafe { pr.ExpandToEnclosingUnit(TextUnit_Paragraph) };
                    let para_text = unsafe { pr.GetText(2000) }
                        .map(|b| b.to_string())
                        .unwrap_or_default();
                    if !para_text.trim().is_empty() {
                        // Locate word inside paragraph to resolve exact sentence boundaries
                        let offset = extracted_word
                            .as_ref()
                            .and_then(|w| para_text.find(w.as_str()))
                            .map(|byte_idx| para_text[..byte_idx].encode_utf16().count())
                            .unwrap_or(0);
                        resolved_sentence = SentenceResolver::resolve(
                            &SentenceWindow::complete_block(&para_text),
                            offset,
                        );
                    }
                }

                if sentence_mode {
                    if let Some(sent) = resolved_sentence {
                        return Ok(ExtractionSnapshot {
                            generation,
                            text: sent.text,
                            context: None,
                            sentence_completeness: Some(sent.completeness),
                            scope: ExtractionScope::Sentence,
                            completeness: sent.completeness,
                            source: ExtractionSource::Accessibility,
                            anchor_rect: word_rect.or_else(|| uia_element_rect_logical(&elem)),
                            pointer,
                            app_id: self.app_id_at_point(pointer),
                        });
                    }
                }

                if let Some(word) = extracted_word {
                    let (ctx_txt, ctx_comp) = match resolved_sentence {
                        Some(sent) => (Some(sent.text), Some(sent.completeness)),
                        None => (None, None),
                    };
                    let compact_elem_rect = uia_element_rect_logical(&elem)
                        .filter(|r| r.width <= 160.0 && r.height <= 44.0);
                    return Ok(ExtractionSnapshot {
                        generation,
                        text: word,
                        context: ctx_txt,
                        sentence_completeness: ctx_comp,
                        scope: ExtractionScope::Word,
                        completeness: ExtractionCompleteness::Complete,
                        source: ExtractionSource::Accessibility,
                        anchor_rect: word_rect.or(compact_elem_rect),
                        pointer,
                        app_id: self.app_id_at_point(pointer),
                    });
                }
            }
        }

        // 2. Short UI control / ValuePattern / Name fallback
        let val_str = unsafe {
            elem.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
                .and_then(|vp| vp.CurrentValue().ok())
                .map(|b| b.to_string())
        };
        let name_str = unsafe { elem.CurrentName().ok().map(|b| b.to_string()) };
        let label = val_str
            .filter(|s| !s.trim().is_empty())
            .or(name_str)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if !label.is_empty() && label.chars().count() <= 180 {
            let scope = if crate::extraction::resolver::is_single_word_label(&label) {
                ExtractionScope::Word
            } else {
                ExtractionScope::Label
            };
            return Ok(ExtractionSnapshot {
                generation,
                text: label,
                context: None,
                sentence_completeness: None,
                scope,
                completeness: ExtractionCompleteness::Complete,
                source: ExtractionSource::Accessibility,
                anchor_rect: uia_element_rect_logical(&elem),
                pointer,
                app_id: self.app_id_at_point(pointer),
            });
        }

        Err(ExtractionFailure::NotSupported)
    }

    fn extract_hover_ocr(
        &self,
        pointer: ScreenPoint,
        sentence_mode: bool,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let mon = self.monitor_bounds_at(pointer);
        let half_w = 500.0;
        let half_h = 170.0;
        let rx = (pointer.x - half_w).max(mon.min_x());
        let ry = (pointer.y - half_h).max(mon.min_y());
        let rw = ((pointer.x + half_w).min(mon.max_x()) - rx).max(0.0);
        let rh = ((pointer.y + half_h).min(mon.max_y()) - ry).max(0.0);
        let region = ScreenRect::new(rx, ry, rw, rh);

        let lines = run_windows_ocr(region)?;
        let hit = OcrClusterer::extract_at_point(pointer, &lines)
            .ok_or(ExtractionFailure::NoText)?;

        if sentence_mode {
            return Ok(ExtractionSnapshot {
                generation,
                text: hit.sentence_text.unwrap_or(hit.word.text),
                context: None,
                sentence_completeness: Some(hit.sentence_completeness),
                scope: ExtractionScope::Sentence,
                completeness: hit.sentence_completeness,
                source: ExtractionSource::Ocr,
                anchor_rect: Some(hit.sentence_rect),
                pointer,
                app_id: self.app_id_at_point(pointer),
            });
        }

        Ok(ExtractionSnapshot {
            generation,
            text: hit.word.text,
            context: hit.sentence_text,
            sentence_completeness: Some(hit.sentence_completeness),
            scope: ExtractionScope::Word,
            completeness: ExtractionCompleteness::Complete,
            source: ExtractionSource::Ocr,
            anchor_rect: Some(hit.word.rect),
            pointer,
            app_id: self.app_id_at_point(pointer),
        })
    }

    fn extract_region_ocr(
        &self,
        rect: ScreenRect,
        generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        let lines = run_windows_ocr(rect)?;
        let text = OcrClusterer::assemble_region_text(&lines).ok_or(ExtractionFailure::NoText)?;
        if !is_allowed_input_text(&text) {
            return Err(ExtractionFailure::TooLong);
        }
        Ok(ExtractionSnapshot {
            generation,
            text,
            context: None,
            sentence_completeness: None,
            scope: ExtractionScope::RegionOcr,
            completeness: ExtractionCompleteness::Complete,
            source: ExtractionSource::Ocr,
            anchor_rect: Some(rect),
            pointer: ScreenPoint::new(rect.mid_x(), rect.max_y()),
            app_id: None,
        })
    }

    fn capture_region_image(
        &self,
        rect: ScreenRect,
        _include_jpeg_preview: bool,
    ) -> Option<CapturedImage> {
        let (px, py, pw, ph) = logical_to_physical_rect(rect);
        let mut buf = capture_screen_bgra(px, py, pw, ph)?;
        for px_chunk in buf.chunks_exact_mut(4) {
            px_chunk.swap(0, 2); // BGRA -> RGBA
            px_chunk[3] = 255;
        }
        Some(CapturedImage {
            rect,
            pixel_width: pw as u32,
            pixel_height: ph as u32,
            rgba: buf,
            preview_jpeg: None,
        })
    }

    fn ocr_captured_image(&self, image: &CapturedImage) -> Result<Vec<OcrLine>, ExtractionFailure> {
        let expected = (image.pixel_width as usize) * (image.pixel_height as usize) * 4;
        if image.pixel_width < 4 || image.pixel_height < 4 || image.rgba.len() < expected {
            return Err(ExtractionFailure::NoText);
        }
        let mut bgra = image.rgba[..expected].to_vec();
        for px_chunk in bgra.chunks_exact_mut(4) {
            px_chunk.swap(0, 2); // RGBA -> BGRA
        }
        run_windows_ocr_bgra(
            &bgra,
            image.pixel_width as i32,
            image.pixel_height as i32,
            image.rect,
        )
    }

    fn permissions_status(&self, _prompt_accessibility: bool) -> PermissionsStatus {
        PermissionsStatus {
            platform: "windows".into(),
            accessibility: true,
            screen_recording: true,
        }
    }

    fn write_clipboard(&self, text: &str) -> Result<(), String> {
        write_clipboard_unicode(text)
    }

    fn configure_popup_window(&self, window: &tauri::WebviewWindow) {
        if let Ok(hwnd_raw) = window.hwnd() {
            let hwnd = HWND(hwnd_raw.0);
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let flags = (WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT)
                    .0 as isize;
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | flags);
            }
        }
    }

    fn present_popup(&self, window: &tauri::WebviewWindow, rect: ScreenRect, interactive: bool) {
        self.set_popup_interactive(window, interactive);
        if let Ok(hwnd_raw) = window.hwnd() {
            let hwnd = HWND(hwnd_raw.0);
            let (px, py, pw, ph) = logical_to_physical_rect(rect);
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    px,
                    py,
                    pw,
                    ph,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }

    fn set_popup_interactive(&self, window: &tauri::WebviewWindow, interactive: bool) {
        if let Ok(hwnd_raw) = window.hwnd() {
            let hwnd = HWND(hwnd_raw.0);
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let trans = WS_EX_TRANSPARENT.0 as isize;
                let base =
                    (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED).0 as isize;
                let next = if interactive {
                    (ex | base) & !trans
                } else {
                    ex | base | trans
                };
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
            }
        }
    }

    fn hide_popup(&self, window: &tauri::WebviewWindow) {
        if let Ok(hwnd_raw) = window.hwnd() {
            let hwnd = HWND(hwnd_raw.0);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}
