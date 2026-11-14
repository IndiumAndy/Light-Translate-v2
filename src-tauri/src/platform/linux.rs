use crate::extraction::{
    is_allowed_input_text, ExtractionCompleteness, ExtractionFailure, ExtractionScope,
    ExtractionSnapshot, ExtractionSource, ScreenPoint, ScreenRect,
};
use crate::platform::{InputEventCallback, PermissionsStatus, PlatformBackend, ShortcutSpec};
use crate::storage::TriggerModifier;
use parking_lot::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask, ModMask, Property,
    WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

const SELECTION_NOTIFY_WAIT: Duration = Duration::from_millis(400);
const SELECTION_INCR_WAIT: Duration = Duration::from_millis(1_500);
const SELECTION_CALL_WAIT: Duration = Duration::from_millis(3_000);
const SHORTCUT_REPEAT_GAP: Duration = Duration::from_millis(350);
const LOCK_MASK: u16 = 1 << 1;
const NUMLOCK_MASK: u16 = 1 << 4;

/// How the current process is attached to a desktop.
///
/// Native X11 can read PRIMARY and grab a global shortcut. XWayland can still
/// read PRIMARY for X11 clients, but grabs do not reach native Wayland windows,
/// so they are not offered. Pure Wayland has neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxDesktopSession {
    X11,
    XWayland,
    Wayland,
    Headless,
}

impl LinuxDesktopSession {
    fn platform_id(self) -> &'static str {
        match self {
            Self::X11 => "linux",
            Self::XWayland => "linux-xwayland",
            Self::Wayland => "linux-wayland",
            Self::Headless => "linux-headless",
        }
    }

    fn has_x11(self) -> bool {
        matches!(self, Self::X11 | Self::XWayland)
    }

    /// `XGrabKey` is reliable on a native X11 session only.
    fn grab_shortcuts(self) -> bool {
        matches!(self, Self::X11)
    }
}

fn detect_linux_session(
    display: Option<&str>,
    wayland_display: Option<&str>,
    session_type: Option<&str>,
) -> LinuxDesktopSession {
    let has_display = display.is_some_and(|value| !value.is_empty());
    let has_wayland = wayland_display.is_some_and(|value| !value.is_empty());
    match session_type
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "x11" => {
            if has_display {
                LinuxDesktopSession::X11
            } else {
                LinuxDesktopSession::Headless
            }
        }
        "wayland" => {
            if has_display {
                LinuxDesktopSession::XWayland
            } else {
                LinuxDesktopSession::Wayland
            }
        }
        _ => {
            if has_display && has_wayland {
                LinuxDesktopSession::XWayland
            } else if has_display {
                LinuxDesktopSession::X11
            } else if has_wayland {
                LinuxDesktopSession::Wayland
            } else {
                LinuxDesktopSession::Headless
            }
        }
    }
}

fn current_linux_session() -> LinuxDesktopSession {
    detect_linux_session(
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OutputGeom {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl OutputGeom {
    fn contains_physical(self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && y >= f64::from(self.y)
            && x < f64::from(self.x) + f64::from(self.width)
            && y < f64::from(self.y) + f64::from(self.height)
    }

    fn to_rect(self) -> ScreenRect {
        ScreenRect::new(
            f64::from(self.x),
            f64::from(self.y),
            f64::from(self.width),
            f64::from(self.height),
        )
    }
}

/// Pick the XRandR output that contains a physical point.
/// Falls back to the nearest output when the point sits in a gap.
fn output_containing(outputs: &[OutputGeom], x: f64, y: f64) -> Option<OutputGeom> {
    if let Some(hit) = outputs
        .iter()
        .copied()
        .find(|output| output.contains_physical(x, y))
    {
        return Some(hit);
    }
    outputs.iter().copied().min_by_key(|output| {
        let cx = f64::from(output.x) + f64::from(output.width) / 2.0;
        let cy = f64::from(output.y) + f64::from(output.height) / 2.0;
        let dx = cx - x;
        let dy = cy - y;
        (dx * dx + dy * dy) as i64
    })
}

fn positive_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale >= 1.0 {
        scale
    } else {
        1.0
    }
}

fn scale_from_gdk_env(raw: &str) -> Option<f64> {
    let value: f64 = raw.trim().parse().ok()?;
    (1.0..=8.0).contains(&value).then_some(value)
}

fn scale_from_xft_dpi(dpi: f64) -> Option<f64> {
    if !(72.0..=800.0).contains(&dpi) {
        return None;
    }
    let scale = (dpi / 96.0).round();
    (1.0..=8.0).contains(&scale).then_some(scale)
}

fn xft_dpi_from_resources(text: &str) -> Option<f64> {
    for line in text.split(['\n', '\0']) {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Xft.dpi") {
            if let Ok(dpi) = value.trim().parse::<f64>() {
                return Some(dpi);
            }
        }
    }
    None
}

fn scale_rect(rect: ScreenRect, scale: f64) -> ScreenRect {
    let factor = positive_scale(scale);
    ScreenRect::new(
        rect.x / factor,
        rect.y / factor,
        rect.width / factor,
        rect.height / factor,
    )
}

/// `rect` was computed in `from_scale` logical space. Tauri positions windows in
/// `to_scale` logical space. Both scales are physical-pixels-per-logical-point.
fn rect_between_scales(rect: ScreenRect, from_scale: f64, to_scale: f64) -> ScreenRect {
    let factor = positive_scale(from_scale) / positive_scale(to_scale);
    ScreenRect::new(
        rect.x * factor,
        rect.y * factor,
        (rect.width * factor).max(1.0),
        (rect.height * factor).max(1.0),
    )
}

struct ScaleCache {
    value: f64,
    locked: bool,
}

impl ScaleCache {
    fn from_env() -> Self {
        if let Some(scale) = std::env::var("GDK_SCALE")
            .ok()
            .as_deref()
            .and_then(scale_from_gdk_env)
        {
            Self {
                value: scale,
                locked: true,
            }
        } else {
            Self {
                value: 1.0,
                locked: false,
            }
        }
    }
}

/// CapsLock and NumLock must not make a grabbed shortcut miss.
fn grab_modifier_variants(base: u16) -> [u16; 4] {
    [
        base,
        base | LOCK_MASK,
        base | NUMLOCK_MASK,
        base | LOCK_MASK | NUMLOCK_MASK,
    ]
}

fn shortcut_base_mask(spec: &ShortcutSpec) -> u16 {
    let mut mask = 0u16;
    if spec.shift {
        mask |= 1 << 0;
    }
    if spec.ctrl {
        mask |= 1 << 2;
    }
    if spec.alt {
        mask |= 1 << 3;
    }
    if spec.meta {
        mask |= 1 << 6;
    }
    mask
}

/// X keycode for the physical key named by a recorded shortcut token.
///
/// The settings recorder stores `KeyboardEvent.code` (`KeyY` → `Y`), which is
/// the evdev position, not the symbol the current layout prints. Xorg's evdev
/// driver numbers those keys as `linux_keycode + 8`.
fn physical_x_keycode(token: &str) -> Option<u8> {
    let linux = linux_evdev_key(token)?;
    u8::try_from(linux.checked_add(8)?).ok()
}

fn linux_evdev_key(token: &str) -> Option<u16> {
    if token.len() == 1 {
        let ch = token.chars().next()?;
        if ch.is_ascii_uppercase() {
            return Some(match ch {
                'A' => 30,
                'B' => 48,
                'C' => 46,
                'D' => 32,
                'E' => 18,
                'F' => 33,
                'G' => 34,
                'H' => 35,
                'I' => 23,
                'J' => 36,
                'K' => 37,
                'L' => 38,
                'M' => 50,
                'N' => 49,
                'O' => 24,
                'P' => 25,
                'Q' => 16,
                'R' => 19,
                'S' => 31,
                'T' => 20,
                'U' => 22,
                'V' => 47,
                'W' => 17,
                'X' => 45,
                'Y' => 21,
                'Z' => 44,
                _ => return None,
            });
        }
        if let Some(digit) = ch.to_digit(10) {
            return Some(if digit == 0 { 11 } else { 1 + digit as u16 });
        }
    }
    if let Some(number) = token
        .strip_prefix('F')
        .and_then(|rest| rest.parse::<u16>().ok())
    {
        return match number {
            1..=10 => Some(58 + number),
            11 => Some(87),
            12 => Some(88),
            _ => None,
        };
    }
    Some(match token {
        "SPACE" => 57,
        "GRAVE" => 41,
        "MINUS" => 12,
        "EQUAL" => 13,
        "BRACKETLEFT" => 26,
        "BRACKETRIGHT" => 27,
        "BACKSLASH" => 43,
        "SEMICOLON" => 39,
        "QUOTE" => 40,
        "COMMA" => 51,
        "PERIOD" => 52,
        "SLASH" => 53,
        _ => return None,
    })
}

/// Prefer the physical key when the server actually has that keycode.
/// Keysym lookup is only the fallback for servers that do not use evdev numbering.
fn choose_shortcut_keycode(
    min_keycode: u8,
    max_keycode: u8,
    physical: Option<u8>,
    keysym_code: Option<u8>,
) -> Option<u8> {
    if let Some(code) = physical {
        if code >= min_keycode && code <= max_keycode {
            return Some(code);
        }
    }
    keysym_code
}

fn shortcut_keysyms(key: &str) -> Vec<u32> {
    if key.len() == 1 {
        let Some(ch) = key.chars().next() else {
            return Vec::new();
        };
        if ch.is_ascii_uppercase() {
            return vec![ch.to_ascii_lowercase() as u32, ch as u32];
        }
        if ch.is_ascii_digit() {
            return vec![ch as u32];
        }
    }
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|rest| rest.parse::<u32>().ok())
    {
        if (1..=12).contains(&number) {
            // XK_F1 is 0xffbe.
            return vec![0xffbe + number - 1];
        }
    }
    let symbol = match key {
        "SPACE" => 0x0020,
        "GRAVE" => 0x0060,
        "MINUS" => 0x002d,
        "EQUAL" => 0x003d,
        "BRACKETLEFT" => 0x005b,
        "BRACKETRIGHT" => 0x005d,
        "BACKSLASH" => 0x005c,
        "SEMICOLON" => 0x003b,
        "QUOTE" => 0x0027,
        "COMMA" => 0x002c,
        "PERIOD" => 0x002e,
        "SLASH" => 0x002f,
        _ => return Vec::new(),
    };
    vec![symbol]
}

fn find_keycode(
    min_keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: &[u32],
    wanted: &[u32],
) -> Option<u8> {
    let per = usize::from(keysyms_per_keycode);
    if per == 0 || wanted.is_empty() {
        return None;
    }
    for (index, chunk) in keysyms.chunks(per).enumerate() {
        if chunk
            .iter()
            .any(|symbol| *symbol != 0 && wanted.contains(symbol))
        {
            let code = usize::from(min_keycode) + index;
            if code <= usize::from(u8::MAX) {
                return Some(code as u8);
            }
        }
    }
    None
}

fn text_from_selection_bytes(bytes: &[u8], latin1: bool) -> Option<String> {
    let owned;
    let text = if latin1 {
        owned = bytes
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>();
        owned.as_str()
    } else {
        owned = String::from_utf8_lossy(bytes).into_owned();
        owned.as_str()
    };
    let trimmed = text
        .trim_matches(|ch: char| ch.is_whitespace() || ch == '\0')
        .to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Append one INCR chunk. An empty chunk is the ICCCM end marker.
fn push_incr_chunk(buffer: &mut Vec<u8>, chunk: &[u8]) -> bool {
    if chunk.is_empty() {
        true
    } else {
        buffer.extend_from_slice(chunk);
        false
    }
}

enum X11Cmd {
    ReadSelection {
        atom_name: Vec<u8>,
        reply: Sender<Option<String>>,
    },
    QueryPointer {
        reply: Sender<Option<(f64, f64)>>,
    },
    MonitorAt {
        x: f64,
        y: f64,
        reply: Sender<Option<ScreenRect>>,
    },
    SetGrab {
        spec: Option<ShortcutSpec>,
        enabled: bool,
    },
}

struct SelRequest {
    atom_name: Vec<u8>,
    reply: Sender<Option<String>>,
}

struct PendingRead {
    request: SelRequest,
    selection: u32,
    deadline: Instant,
    stage: ReadStage,
}

enum ReadStage {
    Notify {
        utf8_done: bool,
    },
    Incr {
        buf: Vec<u8>,
        utf8_done: bool,
        latin1: bool,
    },
}

struct X11State {
    conn: RustConnection,
    root: u32,
    window: u32,
    utf8: u32,
    string_atom: u32,
    incr: u32,
    prop: u32,
    root_rect: ScreenRect,
    outputs: Vec<OutputGeom>,
}

impl X11State {
    fn connect() -> Option<Self> {
        let (conn, screen_num) = RustConnection::connect(None).ok()?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let root_rect = ScreenRect::new(
            0.0,
            0.0,
            f64::from(screen.width_in_pixels),
            f64::from(screen.height_in_pixels),
        );
        let window = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .ok()?
        .check()
        .ok()?;

        let utf8 = intern_atom(&conn, b"UTF8_STRING")?;
        let string_atom = u32::from(AtomEnum::STRING);
        let incr = intern_atom(&conn, b"INCR")?;
        let prop = intern_atom(&conn, b"LT_SEL_PROP")?;
        let outputs = load_outputs(&conn, root).unwrap_or_default();
        Some(Self {
            conn,
            root,
            window,
            utf8,
            string_atom,
            incr,
            prop,
            root_rect,
            outputs,
        })
    }

    fn xft_dpi(&self) -> Option<f64> {
        let atom = intern_atom(&self.conn, b"RESOURCE_MANAGER")?;
        let reply = self
            .conn
            .get_property(false, self.root, atom, AtomEnum::STRING, 0, 262_144)
            .ok()?
            .reply()
            .ok()?;
        let text = String::from_utf8_lossy(&reply.value);
        xft_dpi_from_resources(&text)
    }

    fn query_pointer(&self) -> Option<(f64, f64)> {
        let reply = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((f64::from(reply.root_x), f64::from(reply.root_y)))
    }

    fn refresh_geometry(&mut self) {
        if let Ok(cookie) = self.conn.get_geometry(self.root) {
            if let Ok(geom) = cookie.reply() {
                self.root_rect =
                    ScreenRect::new(0.0, 0.0, f64::from(geom.width), f64::from(geom.height));
            }
        }
        if let Some(outputs) = load_outputs(&self.conn, self.root) {
            self.outputs = outputs;
        }
    }

    fn monitor_at(&mut self, x: f64, y: f64) -> ScreenRect {
        self.refresh_geometry();
        output_containing(&self.outputs, x, y)
            .map(OutputGeom::to_rect)
            .unwrap_or(self.root_rect)
    }

    fn keycode_for(&self, spec: &ShortcutSpec) -> Option<u8> {
        let setup = self.conn.setup();
        choose_shortcut_keycode(
            setup.min_keycode,
            setup.max_keycode,
            physical_x_keycode(&spec.key),
            self.keysym_keycode(spec),
        )
    }

    fn keysym_keycode(&self, spec: &ShortcutSpec) -> Option<u8> {
        let wanted = shortcut_keysyms(&spec.key);
        let setup = self.conn.setup();
        let min = setup.min_keycode;
        let count = setup.max_keycode.saturating_sub(min).saturating_add(1);
        if count == 0 {
            return None;
        }
        let mapping = self
            .conn
            .get_keyboard_mapping(min, count)
            .ok()?
            .reply()
            .ok()?;
        find_keycode(min, mapping.keysyms_per_keycode, &mapping.keysyms, &wanted)
    }

    fn clear_grabs(&self, grabbed: &[(u8, u16)]) {
        for (keycode, modifiers) in grabbed {
            if let Ok(cookie) = self
                .conn
                .ungrab_key(*keycode, self.root, (*modifiers).into())
            {
                let _ = cookie.check();
            }
        }
        let _ = self.conn.flush();
    }

    fn apply_grabs(&self, spec: Option<&ShortcutSpec>) -> Vec<(u8, u16)> {
        let Some(spec) = spec else {
            return Vec::new();
        };
        let Some(keycode) = self.keycode_for(spec) else {
            tracing::warn!(key = %spec.key, "linux shortcut key has no keycode");
            return Vec::new();
        };
        let mut grabbed = Vec::new();
        for modifiers in grab_modifier_variants(shortcut_base_mask(spec)) {
            match self.conn.grab_key(
                false,
                self.root,
                ModMask::from(modifiers),
                keycode,
                x11rb::protocol::xproto::GrabMode::ASYNC,
                x11rb::protocol::xproto::GrabMode::ASYNC,
            ) {
                Ok(cookie) => match cookie.check() {
                    Ok(()) => grabbed.push((keycode, modifiers)),
                    Err(err) => {
                        tracing::warn!(?err, modifiers, "linux shortcut grab rejected");
                    }
                },
                Err(err) => {
                    tracing::warn!(?err, "linux shortcut grab failed to send");
                }
            }
        }
        let _ = self.conn.flush();
        if grabbed.is_empty() {
            tracing::warn!("linux selection shortcut was not grabbed");
        }
        grabbed
    }

    fn begin_convert(&self, selection: u32, target: u32) -> bool {
        match self.conn.convert_selection(
            self.window,
            selection,
            target,
            self.prop,
            x11rb::CURRENT_TIME,
        ) {
            Ok(cookie) => cookie.check().is_ok(),
            Err(err) => {
                tracing::debug!(?err, "convert_selection failed");
                false
            }
        }
    }

    fn take_property(&self) -> Option<(u32, Vec<u8>)> {
        let reply = self
            .conn
            .get_property(true, self.window, self.prop, AtomEnum::ANY, 0, 262_144)
            .ok()?
            .reply()
            .ok()?;
        Some((reply.type_, reply.value))
    }
}

fn intern_atom(conn: &RustConnection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name)
        .ok()?
        .reply()
        .ok()
        .map(|reply| reply.atom)
}

fn load_outputs(conn: &RustConnection, root: u32) -> Option<Vec<OutputGeom>> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())?;
    let mut outputs = Vec::new();
    for crtc in resources.crtcs {
        let Ok(cookie) = conn.randr_get_crtc_info(crtc, resources.config_timestamp) else {
            continue;
        };
        let Ok(info) = cookie.reply() else {
            continue;
        };
        if info.width == 0 || info.height == 0 {
            continue;
        }
        outputs.push(OutputGeom {
            x: i32::from(info.x),
            y: i32::from(info.y),
            width: u32::from(info.width),
            height: u32::from(info.height),
        });
    }
    Some(outputs)
}

struct X11Worker {
    rx: Receiver<X11Cmd>,
    callback: std::sync::Arc<Mutex<Option<InputEventCallback>>>,
    scale: std::sync::Arc<Mutex<ScaleCache>>,
    state: Option<X11State>,
    pending: Option<PendingRead>,
    queue: std::collections::VecDeque<SelRequest>,
    grabbed: Vec<(u8, u16)>,
    shortcut: Option<ShortcutSpec>,
    grabs_enabled: bool,
    last_shortcut_at: Instant,
    last_pointer_sample: Instant,
}

impl X11Worker {
    fn run(mut self) {
        loop {
            let wait = if self.pending.is_some() {
                Duration::from_millis(8)
            } else if self.state.is_none() {
                Duration::from_millis(200)
            } else {
                Duration::from_millis(20)
            };
            match self.rx.recv_timeout(wait) {
                Ok(cmd) => {
                    if !self.handle_cmd(cmd) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.state.is_none() {
                self.try_connect();
            }
            if !self.poll_events() {
                break;
            }
            self.expire_pending();
            self.emit_pointer_sample();
        }
        if let Some(state) = self.state.as_ref() {
            state.clear_grabs(&self.grabbed);
            let _ = state.conn.destroy_window(state.window);
            let _ = state.conn.flush();
        }
    }

    fn try_connect(&mut self) {
        let Some(state) = X11State::connect() else {
            return;
        };
        if !self.scale.lock().locked {
            if let Some(dpi) = state.xft_dpi() {
                if let Some(scale) = scale_from_xft_dpi(dpi) {
                    self.scale.lock().value = scale;
                }
            }
        }
        self.state = Some(state);
        self.regrab();
    }

    fn handle_cmd(&mut self, cmd: X11Cmd) -> bool {
        if self.state.is_none() {
            self.try_connect();
        }
        match cmd {
            X11Cmd::ReadSelection { atom_name, reply } => {
                let request = SelRequest { atom_name, reply };
                if self.state.is_none() {
                    let _ = request.reply.send(None);
                } else if self.pending.is_some() {
                    self.queue.push_back(request);
                } else {
                    let _ = self.start_read(request);
                }
            }
            X11Cmd::QueryPointer { reply } => {
                let point = self.state.as_ref().and_then(X11State::query_pointer);
                let _ = reply.send(point);
            }
            X11Cmd::MonitorAt { x, y, reply } => {
                let rect = self.state.as_mut().map(|state| state.monitor_at(x, y));
                let _ = reply.send(rect);
            }
            X11Cmd::SetGrab { spec, enabled } => {
                self.shortcut = spec;
                self.grabs_enabled = enabled;
                self.regrab();
            }
        }
        true
    }

    fn regrab(&mut self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        state.clear_grabs(&self.grabbed);
        self.grabbed.clear();
        if self.grabs_enabled {
            let spec = self.shortcut.clone();
            self.grabbed = state.apply_grabs(spec.as_ref());
        }
    }

    fn start_read(&mut self, request: SelRequest) -> bool {
        let Some(state) = self.state.as_ref() else {
            let _ = request.reply.send(None);
            return false;
        };
        let Some(selection) = intern_atom(&state.conn, &request.atom_name) else {
            let _ = request.reply.send(None);
            return false;
        };
        let target = state.utf8;
        if !state.begin_convert(selection, target) {
            let _ = request.reply.send(None);
            return false;
        }
        self.pending = Some(PendingRead {
            request,
            selection,
            deadline: Instant::now() + SELECTION_NOTIFY_WAIT,
            stage: ReadStage::Notify { utf8_done: false },
        });
        true
    }

    fn poll_events(&mut self) -> bool {
        loop {
            let polled = self.state.as_ref().map(|state| state.conn.poll_for_event());
            match polled {
                None | Some(Ok(None)) => return true,
                Some(Ok(Some(event))) => self.handle_event(event),
                Some(Err(err)) => {
                    tracing::warn!(?err, "linux x11 connection dropped");
                    self.fail_pending();
                    self.grabbed.clear();
                    self.state = None;
                    return true;
                }
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::SelectionNotify(ev) => self.on_selection_notify(ev.property),
            Event::PropertyNotify(ev) => {
                if ev.state == Property::NEW_VALUE {
                    let prop = self.state.as_ref().map(|state| state.prop);
                    if prop == Some(ev.atom) {
                        self.on_incr_chunk();
                    }
                }
            }
            Event::KeyPress(ev) => self.on_key_press(ev.detail, ev.state, ev.root_x, ev.root_y),
            _ => {}
        }
    }

    fn on_key_press(&mut self, detail: u8, state: KeyButMask, root_x: i16, root_y: i16) {
        if self.grabbed.is_empty() || !self.grabbed.iter().any(|(code, _)| *code == detail) {
            return;
        }
        let modifiers = u16::from(state) & !(LOCK_MASK | NUMLOCK_MASK | (0b11111 << 8));
        let matched = self.grabbed.iter().any(|(code, mask)| {
            *code == detail && (*mask & !(LOCK_MASK | NUMLOCK_MASK)) == modifiers
        });
        if !matched || self.last_shortcut_at.elapsed() < SHORTCUT_REPEAT_GAP {
            return;
        }
        self.last_shortcut_at = Instant::now();
        let scale = positive_scale(self.scale.lock().value);
        let pointer = ScreenPoint::new(f64::from(root_x) / scale, f64::from(root_y) / scale);
        let callback = self.callback.lock().clone();
        if let Some(callback) = callback {
            callback(crate::platform::PlatformInputEvent::ShortcutSelection { pointer });
        }
    }

    fn on_selection_notify(&mut self, property: u32) {
        let waiting = matches!(
            self.pending.as_ref().map(|pending| &pending.stage),
            Some(ReadStage::Notify { .. })
        );
        if !waiting {
            return;
        }
        if property == u32::from(AtomEnum::NONE) {
            self.finish_or_fallback(None);
            return;
        }
        let incr_atom = self.state.as_ref().map(|state| state.incr);
        let string_atom = self.state.as_ref().map(|state| state.string_atom);
        let property_body = self.state.as_ref().and_then(X11State::take_property);
        let Some((type_atom, data)) = property_body else {
            self.finish_or_fallback(None);
            return;
        };
        if Some(type_atom) == incr_atom {
            if let Some(pending) = self.pending.as_mut() {
                let utf8_done = match pending.stage {
                    ReadStage::Notify { utf8_done } | ReadStage::Incr { utf8_done, .. } => {
                        utf8_done
                    }
                };
                pending.stage = ReadStage::Incr {
                    buf: Vec::new(),
                    utf8_done,
                    latin1: false,
                };
                pending.deadline = Instant::now() + SELECTION_INCR_WAIT;
            }
            return;
        }
        let latin1 = Some(type_atom) == string_atom;
        self.finish_or_fallback(text_from_selection_bytes(&data, latin1));
    }

    fn on_incr_chunk(&mut self) {
        // PropertyNotify also arrives when the selection owner first writes the
        // property, before SelectionNotify. Reading it here would delete the
        // bytes the notify handler still needs.
        let in_incr = matches!(
            self.pending.as_ref().map(|pending| &pending.stage),
            Some(ReadStage::Incr { .. })
        );
        if !in_incr {
            return;
        }
        let incr_atom = self.state.as_ref().map(|state| state.incr);
        let string_atom = self.state.as_ref().map(|state| state.string_atom);
        let property_body = self.state.as_ref().and_then(X11State::take_property);
        let Some((type_atom, data)) = property_body else {
            return;
        };
        if Some(type_atom) == incr_atom {
            return;
        }
        let chunk_is_latin1 = Some(type_atom) == string_atom;
        let finished = match self.pending.as_mut() {
            Some(PendingRead {
                stage: ReadStage::Incr { buf, latin1, .. },
                deadline,
                ..
            }) => {
                if chunk_is_latin1 {
                    *latin1 = true;
                }
                let done = push_incr_chunk(buf, &data);
                if !done {
                    *deadline = Instant::now() + SELECTION_INCR_WAIT;
                }
                done
            }
            _ => return,
        };
        if finished {
            let text = self
                .pending
                .as_ref()
                .and_then(|pending| match &pending.stage {
                    ReadStage::Incr { buf, latin1, .. } => text_from_selection_bytes(buf, *latin1),
                    ReadStage::Notify { .. } => None,
                });
            self.finish_or_fallback(text);
        }
    }

    fn finish_or_fallback(&mut self, text: Option<String>) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let utf8_done = match pending.stage {
            ReadStage::Notify { utf8_done } | ReadStage::Incr { utf8_done, .. } => utf8_done,
        };
        if text.is_none() && !utf8_done {
            let selection = pending.selection;
            let target = self.state.as_ref().map(|state| state.string_atom);
            if let Some(target) = target {
                if self
                    .state
                    .as_ref()
                    .is_some_and(|state| state.begin_convert(selection, target))
                {
                    if let Some(pending) = self.pending.as_mut() {
                        pending.stage = ReadStage::Notify { utf8_done: true };
                        pending.deadline = Instant::now() + SELECTION_NOTIFY_WAIT;
                    }
                    return;
                }
            }
        }
        if let Some(pending) = self.pending.take() {
            let _ = pending.request.reply.send(text);
        }
        self.pump_queue();
    }

    fn emit_pointer_sample(&mut self) {
        if self.last_pointer_sample.elapsed() < Duration::from_millis(50) {
            return;
        }
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let Some((x, y)) = state.query_pointer() else {
            return;
        };
        self.last_pointer_sample = Instant::now();
        let scale = positive_scale(self.scale.lock().value);
        let pointer = ScreenPoint::new(x / scale, y / scale);
        let callback = self.callback.lock().clone();
        if let Some(callback) = callback {
            callback(crate::platform::PlatformInputEvent::PointerMoved { pointer });
        }
    }

    fn expire_pending(&mut self) {
        let expired = self
            .pending
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.deadline);
        if !expired {
            return;
        }
        let partial = self
            .pending
            .as_ref()
            .and_then(|pending| match &pending.stage {
                ReadStage::Incr { buf, latin1, .. } if !buf.is_empty() => {
                    text_from_selection_bytes(buf, *latin1)
                }
                _ => None,
            });
        self.finish_or_fallback(partial);
    }

    fn pump_queue(&mut self) {
        while self.pending.is_none() {
            let Some(request) = self.queue.pop_front() else {
                return;
            };
            if self.state.is_none() {
                let _ = request.reply.send(None);
                continue;
            }
            let _ = self.start_read(request);
        }
    }

    fn fail_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            let _ = pending.request.reply.send(None);
        }
        while let Some(request) = self.queue.pop_front() {
            let _ = request.reply.send(None);
        }
    }
}

#[derive(Clone)]
struct X11Client {
    tx: Sender<X11Cmd>,
}

impl X11Client {
    fn spawn(
        callback: std::sync::Arc<Mutex<Option<InputEventCallback>>>,
        scale: std::sync::Arc<Mutex<ScaleCache>>,
    ) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let worker = X11Worker {
            rx,
            callback,
            scale,
            state: None,
            pending: None,
            queue: std::collections::VecDeque::new(),
            grabbed: Vec::new(),
            shortcut: None,
            grabs_enabled: false,
            last_shortcut_at: Instant::now() - SHORTCUT_REPEAT_GAP,
            last_pointer_sample: Instant::now() - Duration::from_millis(50),
        };
        thread::Builder::new()
            .name("lt-x11".into())
            .spawn(move || worker.run())
            .ok()?;
        Some(Self { tx })
    }

    fn read_selection(&self, atom_name: &[u8]) -> Option<String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(X11Cmd::ReadSelection {
                atom_name: atom_name.to_vec(),
                reply: reply_tx,
            })
            .ok()?;
        reply_rx.recv_timeout(SELECTION_CALL_WAIT).ok().flatten()
    }

    fn query_pointer(&self) -> Option<(f64, f64)> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(X11Cmd::QueryPointer { reply: reply_tx })
            .ok()?;
        reply_rx
            .recv_timeout(Duration::from_millis(500))
            .ok()
            .flatten()
    }

    fn monitor_at(&self, x: f64, y: f64) -> Option<ScreenRect> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(X11Cmd::MonitorAt {
                x,
                y,
                reply: reply_tx,
            })
            .ok()?;
        reply_rx
            .recv_timeout(Duration::from_millis(500))
            .ok()
            .flatten()
    }

    fn set_grab(&self, spec: Option<ShortcutSpec>, enabled: bool) {
        let _ = self.tx.send(X11Cmd::SetGrab { spec, enabled });
    }
}

pub struct LinuxPlatform {
    callback: std::sync::Arc<Mutex<Option<InputEventCallback>>>,
    scale: std::sync::Arc<Mutex<ScaleCache>>,
    session: LinuxDesktopSession,
    x11: Mutex<Option<X11Client>>,
    selection_shortcut: Mutex<Option<ShortcutSpec>>,
    grab_paused: Mutex<bool>,
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxPlatform {
    pub fn new() -> Self {
        Self {
            callback: std::sync::Arc::new(Mutex::new(None)),
            scale: std::sync::Arc::new(Mutex::new(ScaleCache::from_env())),
            session: current_linux_session(),
            x11: Mutex::new(None),
            selection_shortcut: Mutex::new(None),
            grab_paused: Mutex::new(false),
            clipboard: Mutex::new(None),
        }
    }

    fn scale_value(&self) -> f64 {
        positive_scale(self.scale.lock().value)
    }

    fn ensure_x11(&self) -> Option<X11Client> {
        if !self.session.has_x11() {
            return None;
        }
        let mut guard = self.x11.lock();
        if let Some(client) = guard.as_ref() {
            return Some(client.clone());
        }
        let client = X11Client::spawn(
            std::sync::Arc::clone(&self.callback),
            std::sync::Arc::clone(&self.scale),
        )?;
        client.set_grab(self.selection_shortcut.lock().clone(), self.grabs_active());
        *guard = Some(client.clone());
        Some(client)
    }

    fn grabs_active(&self) -> bool {
        self.session.grab_shortcuts() && !*self.grab_paused.lock()
    }

    fn sync_grab(&self) {
        if let Some(client) = self.x11.lock().clone() {
            client.set_grab(self.selection_shortcut.lock().clone(), self.grabs_active());
        }
    }
}

impl PlatformBackend for LinuxPlatform {
    fn start_input_monitor(&self, callback: InputEventCallback) {
        *self.callback.lock() = Some(callback);
        let _ = self.ensure_x11();
    }

    fn update_trigger_config(
        &self,
        _modifier: TriggerModifier,
        _double_tap_selection: bool,
        selection_shortcut: &str,
        _capture_shortcut: &str,
    ) {
        *self.selection_shortcut.lock() = ShortcutSpec::parse(selection_shortcut);
        if self.session.grab_shortcuts() {
            let _ = self.ensure_x11();
        }
        self.sync_grab();
    }

    fn set_shortcut_recording(&self, active: bool) {
        *self.grab_paused.lock() = active;
        self.sync_grab();
    }

    fn current_pointer(&self) -> ScreenPoint {
        let scale = self.scale_value();
        if let Some(client) = self.ensure_x11() {
            if let Some((x, y)) = client.query_pointer() {
                return ScreenPoint::new(x / scale, y / scale);
            }
        }
        ScreenPoint::new(400.0, 300.0)
    }

    fn monitor_bounds_at(&self, point: ScreenPoint) -> ScreenRect {
        let scale = self.scale_value();
        if let Some(client) = self.ensure_x11() {
            if let Some(physical) = client.monitor_at(point.x * scale, point.y * scale) {
                return scale_rect(physical, scale);
            }
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
        if !self.session.has_x11() {
            return Err(ExtractionFailure::NotSupported);
        }
        let Some(client) = self.ensure_x11() else {
            return Err(ExtractionFailure::NotSupported);
        };
        let Some(text) = client.read_selection(b"PRIMARY") else {
            return Ok(None);
        };
        if !is_allowed_input_text(&text) {
            return Err(ExtractionFailure::TooLong);
        }
        Ok(Some(ExtractionSnapshot {
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
        }))
    }

    fn extract_hover_accessibility(
        &self,
        _pointer: ScreenPoint,
        _sentence_mode: bool,
        _generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        Err(ExtractionFailure::NotSupported)
    }

    fn extract_hover_ocr(
        &self,
        _pointer: ScreenPoint,
        _sentence_mode: bool,
        _generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        Err(ExtractionFailure::NotSupported)
    }

    fn extract_region_ocr(
        &self,
        _rect: ScreenRect,
        _generation: u64,
    ) -> Result<ExtractionSnapshot, ExtractionFailure> {
        Err(ExtractionFailure::NotSupported)
    }

    fn permissions_status(&self, _prompt_accessibility: bool) -> PermissionsStatus {
        // Linux has no Accessibility or Screen Recording permission that this
        // process can grant. Reporting them as on whenever DISPLAY is set made
        // XWayland look fully authorized.
        PermissionsStatus {
            platform: self.session.platform_id().into(),
            accessibility: false,
            screen_recording: false,
        }
    }

    fn write_clipboard(&self, text: &str) -> Result<(), String> {
        let mut guard = self.clipboard.lock();
        if guard.is_none() {
            *guard = Some(open_clipboard()?);
        }
        let Some(clipboard) = guard.as_mut() else {
            return Err("剪贴板不可用".into());
        };
        match clipboard.set_text(text.to_string()) {
            Ok(()) => Ok(()),
            Err(_) => {
                let mut fresh = open_clipboard()?;
                fresh
                    .set_text(text.to_string())
                    .map_err(|err| format!("写入剪贴板失败: {err}"))?;
                *guard = Some(fresh);
                Ok(())
            }
        }
    }

    fn configure_popup_window(&self, window: &tauri::WebviewWindow) {
        if let Ok(scale) = window.scale_factor() {
            if scale.is_finite() && scale >= 1.0 {
                let mut cache = self.scale.lock();
                cache.value = scale;
                cache.locked = true;
            }
        }
    }

    fn present_popup(&self, window: &tauri::WebviewWindow, rect: ScreenRect, _interactive: bool) {
        let window_scale = window
            .scale_factor()
            .ok()
            .filter(|scale| scale.is_finite() && *scale >= 1.0)
            .unwrap_or(1.0);
        let coord_scale = self.scale_value();
        let placed = rect_between_scales(rect, coord_scale, window_scale);
        {
            let mut cache = self.scale.lock();
            cache.value = window_scale;
            cache.locked = true;
        }
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            placed.x, placed.y,
        )));
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            placed.width,
            placed.height,
        )));
        let _ = window.show();
    }

    fn set_popup_interactive(&self, _window: &tauri::WebviewWindow, _interactive: bool) {}

    fn hide_popup(&self, window: &tauri::WebviewWindow) {
        let _ = window.hide();
    }
}

fn open_clipboard() -> Result<arboard::Clipboard, String> {
    arboard::Clipboard::new().map_err(|err| format!("剪贴板不可用: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::PlatformBackend;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use x11rb::protocol::xproto::{
        ChangeWindowAttributesAux, PropMode, SelectionNotifyEvent, SELECTION_NOTIFY_EVENT,
    };
    use x11rb::wrapper::ConnectionExt as _;

    #[test]
    fn session_detection_matches_x11_xwayland_and_wayland() {
        assert_eq!(
            detect_linux_session(Some(":0"), None, Some("x11")),
            LinuxDesktopSession::X11
        );
        assert_eq!(
            detect_linux_session(Some(":0"), None, None),
            LinuxDesktopSession::X11
        );
        assert_eq!(
            detect_linux_session(Some(":0"), Some("wayland-0"), Some("x11")),
            LinuxDesktopSession::X11
        );
        assert_eq!(
            detect_linux_session(Some(":0"), Some("wayland-0"), Some("wayland")),
            LinuxDesktopSession::XWayland
        );
        assert_eq!(
            detect_linux_session(Some(":0"), Some("wayland-0"), None),
            LinuxDesktopSession::XWayland
        );
        assert_eq!(
            detect_linux_session(None, Some("wayland-0"), Some("wayland")),
            LinuxDesktopSession::Wayland
        );
        assert_eq!(
            detect_linux_session(None, None, None),
            LinuxDesktopSession::Headless
        );
        assert!(!LinuxDesktopSession::Wayland.grab_shortcuts());
        assert!(!LinuxDesktopSession::XWayland.grab_shortcuts());
        assert!(LinuxDesktopSession::X11.grab_shortcuts());
        assert!(!LinuxDesktopSession::Wayland.has_x11());
        assert_eq!(
            LinuxDesktopSession::XWayland.platform_id(),
            "linux-xwayland"
        );
    }

    #[test]
    fn scale_conversion_uses_gdk_scale_and_xft_dpi() {
        assert_eq!(scale_from_gdk_env("2"), Some(2.0));
        assert_eq!(scale_from_gdk_env("0.5"), None);
        assert_eq!(scale_from_xft_dpi(192.0), Some(2.0));
        assert_eq!(
            xft_dpi_from_resources("Xft.antialias: 1\nXft.dpi: 144\n"),
            Some(144.0)
        );
        let physical = ScreenRect::new(200.0, 100.0, 400.0, 200.0);
        assert_eq!(
            scale_rect(physical, 2.0),
            ScreenRect::new(100.0, 50.0, 200.0, 100.0)
        );
        assert_eq!(
            rect_between_scales(ScreenRect::new(200.0, 100.0, 360.0, 110.0), 1.0, 2.0),
            ScreenRect::new(100.0, 50.0, 180.0, 55.0)
        );
        assert_eq!(
            rect_between_scales(ScreenRect::new(100.0, 40.0, 180.0, 60.0), 2.0, 2.0),
            ScreenRect::new(100.0, 40.0, 180.0, 60.0)
        );
    }

    #[test]
    fn monitor_pick_uses_the_output_under_the_pointer() {
        let outputs = [
            OutputGeom {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            OutputGeom {
                x: 1920,
                y: 0,
                width: 1280,
                height: 1024,
            },
        ];
        let right = output_containing(&outputs, 2000.0, 40.0).unwrap();
        assert_eq!(right.x, 1920);
        assert_eq!(right.width, 1280);
        let left = output_containing(&outputs, 10.0, 10.0).unwrap();
        assert_eq!(left.width, 1920);
        assert!(output_containing(&[], 1.0, 1.0).is_none());
    }

    #[test]
    fn incr_chunks_join_until_the_empty_marker() {
        let mut buffer = Vec::new();
        assert!(!push_incr_chunk(&mut buffer, b"light-"));
        assert!(!push_incr_chunk(&mut buffer, b"translate"));
        assert!(push_incr_chunk(&mut buffer, b""));
        assert_eq!(
            text_from_selection_bytes(&buffer, false).as_deref(),
            Some("light-translate")
        );
        assert_eq!(text_from_selection_bytes(b"   \0  ", false), None);
        assert_eq!(
            text_from_selection_bytes(&[b'c', b'a', b'f', 0xe9], true).as_deref(),
            Some("café")
        );
    }

    #[test]
    fn recorded_shortcut_uses_the_physical_evdev_key() {
        assert_eq!(physical_x_keycode("T"), Some(28));
        assert_eq!(physical_x_keycode("Y"), Some(29));
        assert_eq!(physical_x_keycode("A"), Some(38));
        assert_eq!(physical_x_keycode("F4"), Some(70));
        // QWERTZ: the recorded physical Y must win over the keysym that currently prints Y.
        assert_eq!(
            choose_shortcut_keycode(8, 255, Some(29), Some(52)),
            Some(29)
        );
        assert_eq!(choose_shortcut_keycode(8, 20, None, Some(12)), Some(12));
    }

    #[test]
    fn shortcut_grab_ignores_lock_modifiers_and_maps_keysyms() {
        let spec = ShortcutSpec::parse("Alt+Shift+T").unwrap();
        let base = shortcut_base_mask(&spec);
        assert_eq!(base, (1 << 0) | (1 << 3));
        let variants = grab_modifier_variants(base);
        assert!(variants.contains(&(base | LOCK_MASK)));
        assert!(variants.contains(&(base | NUMLOCK_MASK)));
        assert!(shortcut_keysyms("T").contains(&0x0074));
        assert!(shortcut_keysyms("T").contains(&0x0054));
        assert_eq!(shortcut_keysyms("F4"), vec![0xffc1]);
        let mapping = [0x0074u32, 0x0054, 0, 0];
        assert_eq!(
            find_keycode(8, 2, &mapping, &shortcut_keysyms("T")),
            Some(8)
        );
        assert!(find_keycode(8, 2, &mapping, &shortcut_keysyms("Q")).is_none());
    }

    #[test]
    fn permissions_are_not_implied_by_display() {
        let platform = LinuxPlatform::new();
        let status = platform.permissions_status(true);
        assert!(status.platform.starts_with("linux"));
        assert!(!status.accessibility);
        assert!(!status.screen_recording);
    }

    #[test]
    fn region_ocr_does_not_capture() {
        let platform = LinuxPlatform::new();
        let err = platform
            .extract_region_ocr(ScreenRect::new(0.0, 0.0, 32.0, 32.0), 1)
            .expect_err("region ocr stays unsupported");
        assert_eq!(err, ExtractionFailure::NotSupported);
        assert_eq!(
            platform
                .extract_hover_accessibility(ScreenPoint::new(1.0, 1.0), false, 1)
                .expect_err("hover stays unsupported"),
            ExtractionFailure::NotSupported
        );
    }

    #[test]
    fn write_clipboard_uses_the_system_clipboard_when_a_session_exists() {
        let platform = LinuxPlatform::new();
        match platform.write_clipboard("lt-v2-clipboard-probe") {
            Ok(()) => {
                let mut reader = arboard::Clipboard::new().expect("clipboard reader");
                let got = reader.get_text().unwrap_or_default();
                assert_eq!(got, "lt-v2-clipboard-probe");
            }
            Err(err) => {
                assert!(!err.is_empty(), "clipboard failure should say why");
            }
        }
    }

    fn x11_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn x11_available() -> bool {
        RustConnection::connect(None).is_ok()
    }

    fn serve_selection(
        atom_name: &'static [u8],
        payload: &'static [u8],
        incremental: bool,
        stop: Arc<AtomicBool>,
        ready: Sender<()>,
    ) {
        let Ok((conn, screen_num)) = RustConnection::connect(None) else {
            return;
        };
        let root = conn.setup().roots[screen_num].root;
        let Ok(window) = conn.generate_id() else {
            return;
        };
        let Ok(cookie) = conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        ) else {
            return;
        };
        if cookie.check().is_err() {
            return;
        }
        let Some(selection) = intern_atom(&conn, atom_name) else {
            return;
        };
        let Some(utf8) = intern_atom(&conn, b"UTF8_STRING") else {
            return;
        };
        let Some(incr) = intern_atom(&conn, b"INCR") else {
            return;
        };
        let Ok(cookie) = conn.set_selection_owner(window, selection, x11rb::CURRENT_TIME) else {
            return;
        };
        if cookie.check().is_err() {
            return;
        }
        let _ = ready.send(());

        while !stop.load(Ordering::Relaxed) {
            let event = match conn.poll_for_event() {
                Ok(Some(event)) => event,
                Ok(None) => {
                    thread::sleep(Duration::from_millis(4));
                    continue;
                }
                Err(_) => break,
            };
            let Event::SelectionRequest(request) = event else {
                continue;
            };
            if incremental
                && (request.target == utf8 || request.target == u32::from(AtomEnum::STRING))
            {
                let _ = conn.change_window_attributes(
                    request.requestor,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                );
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    request.property,
                    incr,
                    &[payload.len() as u32],
                );
                send_selection_notify(&conn, &request, request.property);
                let _ = conn.flush();
                serve_incr_chunks(&conn, &request, utf8, payload, &stop);
            } else if request.target == utf8 || request.target == u32::from(AtomEnum::STRING) {
                let _ = conn.change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    request.property,
                    utf8,
                    payload,
                );
                send_selection_notify(&conn, &request, request.property);
                let _ = conn.flush();
            } else {
                send_selection_notify(&conn, &request, u32::from(AtomEnum::NONE));
                let _ = conn.flush();
            }
        }
        let _ = conn.destroy_window(window);
        let _ = conn.flush();
    }

    fn send_selection_notify(
        conn: &RustConnection,
        request: &x11rb::protocol::xproto::SelectionRequestEvent,
        property: u32,
    ) {
        let event = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property,
        };
        let _ = conn.send_event(false, request.requestor, EventMask::NO_EVENT, event);
    }

    fn serve_incr_chunks(
        conn: &RustConnection,
        request: &x11rb::protocol::xproto::SelectionRequestEvent,
        utf8: u32,
        payload: &[u8],
        stop: &AtomicBool,
    ) {
        let mid = (payload.len() / 2).max(1).min(payload.len());
        let chunks: [&[u8]; 3] = [&payload[..mid], &payload[mid..], b""];
        for chunk in chunks {
            let deadline = Instant::now() + Duration::from_millis(800);
            loop {
                if stop.load(Ordering::Relaxed) || Instant::now() > deadline {
                    return;
                }
                match conn.poll_for_event() {
                    Ok(Some(Event::PropertyNotify(ev)))
                        if ev.window == request.requestor
                            && ev.atom == request.property
                            && ev.state == Property::DELETE =>
                    {
                        let _ = conn.change_property8(
                            PropMode::REPLACE,
                            request.requestor,
                            request.property,
                            utf8,
                            chunk,
                        );
                        let _ = conn.flush();
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => thread::sleep(Duration::from_millis(4)),
                    Err(_) => return,
                }
            }
        }
    }

    #[test]
    fn reads_direct_and_incr_primary_on_x11() {
        let _guard = x11_test_lock();
        if !x11_available() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let owner_stop = Arc::clone(&stop);
        let (ready_tx, ready_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            serve_selection(b"PRIMARY", b"luminance", false, owner_stop, ready_tx);
        });
        if ready_rx.recv_timeout(Duration::from_secs(2)).is_err() {
            stop.store(true, Ordering::Relaxed);
            let _ = owner.join();
            panic!("selection owner did not become ready");
        }
        let platform = LinuxPlatform::new();
        let snap = platform
            .extract_selection(ScreenPoint::new(12.0, 12.0), 7, false)
            .expect("x11 selection read")
            .expect("primary text");
        assert_eq!(snap.text, "luminance");
        assert_eq!(snap.source, ExtractionSource::PrimarySelection);
        assert_eq!(snap.generation, 7);
        drop(platform);
        stop.store(true, Ordering::Relaxed);
        let _ = owner.join();

        let stop = Arc::new(AtomicBool::new(false));
        let owner_stop = Arc::clone(&stop);
        let (ready_tx, ready_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            serve_selection(b"PRIMARY", b"light-translate", true, owner_stop, ready_tx);
        });
        if ready_rx.recv_timeout(Duration::from_secs(2)).is_err() {
            stop.store(true, Ordering::Relaxed);
            let _ = owner.join();
            panic!("incr selection owner did not become ready");
        }
        let platform = LinuxPlatform::new();
        let snap = platform
            .extract_selection(ScreenPoint::new(4.0, 4.0), 8, true)
            .expect("incr selection read")
            .expect("incr primary text");
        assert_eq!(snap.text, "light-translate");
        drop(platform);
        stop.store(true, Ordering::Relaxed);
        let _ = owner.join();
    }
}
