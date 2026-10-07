use crate::settings::SettingsWindow;
use cursorcue_config::{CursorStyle, Settings, Smoothing, appdata_config_path};
use cursorcue_core::{Cursor, Point, map_to_source};
use cursorcue_platform_windows::Capture;
use cursorcue_render::Renderer;
use std::{
    mem::size_of,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Dwm::*,
            Dxgi::{DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET},
            Gdi::*,
        },
        System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW, WinRT::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{BOOL, PCWSTR, Result, w},
};

static COMMANDS: OnceLock<SyncSender<u32>> = OnceLock::new();
static CAPTURING: AtomicBool = AtomicBool::new(false);
static DRAW_PENDING: AtomicBool = AtomicBool::new(false);
static REPAINT_PENDING: AtomicBool = AtomicBool::new(false);
static INPUT_OVERFLOW: AtomicBool = AtomicBool::new(false);
const CHOOSE: u32 = 10;
const QUIT: u32 = 11;
const TRAY: u32 = 12;
const SETTINGS: u32 = 19;
const HELP: u32 = 22;
const STOP: u32 = 18;

const TRAY_MESSAGE: u32 = WM_APP + 1;
pub(crate) const SAVE_SETTINGS: u32 = 20;
pub(crate) const RESET_SETTINGS: u32 = 21;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn shortcut_status(settings: &Settings, actual: &[cursorcue_config::Hotkey; 5]) -> String {
    let actions = ["Freeze", "Hide/reveal", "Resume", "Drop", "Toggle"];
    let unavailable: Vec<_> = settings
        .hotkeys
        .iter()
        .zip(actual)
        .zip(actions)
        .filter(|((configured, registered), _)| configured.key != 0 && configured != registered)
        .map(|(_, action)| action)
        .collect();
    if unavailable.is_empty() {
        "Shortcuts are available. Clear a key to disable that shortcut.".into()
    } else {
        format!(
            "Unavailable: {}. Another app uses these keys. Choose different modifiers/keys or clear them. Cursor size and appearance can still be saved.",
            unavailable.join(", ")
        )
    }
}
fn show_guide(owner: HWND) {
    unsafe {
        MessageBoxW(
            Some(owner),
            w!(
                "1. Open the app you want to share. Keep it unminimized.\n2. Tools > Choose window: select that app.\n3. In Meet/Zoom/Teams, share A WINDOW > CursorCue Share.\n4. Work in the ORIGINAL app. Freeze/hide affects only the shared cursor.\n\nCursor size & shortcuts:\nTools > Cursor size & shortcuts. Cursor size is 50-300%; shortcut keys are in the lower section. Apply saves changes; Close leaves the window.\n\nIf a shortcut is unavailable, use another modifier/key or clear it. Menu commands always work. Run one copy of CursorCue.\n\nKeep the source and CursorCue windows unminimized. At frame edges, the shared cursor is inset slightly so its whole shape stays visible.\n\nClosing the CursorCue window stops sharing output and leaves CursorCue in the tray. Choose Quit CursorCue to exit.\n\nThis is an unsigned developer preview. Test your meeting setup before a call or screen-sharing session."
            ),
            w!("CursorCue - quick start"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
struct InstanceGuard(HANDLE);
impl InstanceGuard {
    fn acquire(name: &str) -> Result<Option<Self>> {
        let name = wide(name);
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle)?;
                Ok(None)
            } else {
                Ok(Some(Self(handle)))
            }
        }
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub(crate) fn command(value: u32) {
    if let Some(sender) = COMMANDS.get()
        && sender.try_send(value).is_err()
    {
        INPUT_OVERFLOW.store(true, Ordering::Relaxed);
    }
}
pub fn report_error(text: &str) {
    let text = wide(text);
    // SAFETY: strings are terminated and remain alive throughout this blocking Windows dialog.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("CursorCue"),
            MB_OK | MB_ICONERROR,
        );
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows invokes this procedure with a live HWND; paint structs and string slices live across calls.
    unsafe {
        match message {
            WM_GETMINMAXINFO if lp.0 != 0 => {
                let info = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                info.ptMinTrackSize = POINT {
                    x: 360 * dpi / 96,
                    y: 280 * dpi / 96,
                };
                LRESULT(0)
            }
            WM_SIZE => {
                REPAINT_PENDING.store(true, Ordering::Relaxed);
                crate::presentation::with_window(hwnd, |ui| ui.resize());
                LRESULT(0)
            }
            WM_DPICHANGED => {
                crate::presentation::with_window(hwnd, |ui| {
                    if let Err(error) = ui.update_dpi((wp.0 & 0xffff) as u32) {
                        report_error(&error.to_string());
                    }
                });
                if lp.0 != 0 {
                    let rect = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL if !CAPTURING.load(Ordering::Relaxed) => LRESULT(0),
            WM_VSCROLL if !CAPTURING.load(Ordering::Relaxed) => {
                crate::presentation::with_window(hwnd, |ui| ui.scroll((wp.0 & 0xffff) as i32));
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                if let Some(brush) = crate::presentation::with_window(hwnd, |ui| {
                    ui.static_color(HDC(wp.0 as *mut _), HWND(lp.0 as *mut _))
                }) {
                    LRESULT(brush.0 as isize)
                } else {
                    DefWindowProcW(hwnd, message, wp, lp)
                }
            }
            WM_HOTKEY => {
                command(wp.0 as u32);
                let _ = EndMenu();
                LRESULT(0)
            }
            WM_COMMAND => {
                command((wp.0 & 0xffff) as u32);
                LRESULT(0)
            }
            WM_TIMER => {
                DRAW_PENDING.store(true, Ordering::Relaxed);
                LRESULT(0)
            }
            TRAY_MESSAGE => {
                if lp.0 as u32 == WM_RBUTTONUP || lp.0 as u32 == WM_LBUTTONUP {
                    command(TRAY);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                command(STOP);
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                if !CAPTURING.load(Ordering::Relaxed) {
                    crate::presentation::with_window(hwnd, |ui| ui.erase(HDC(wp.0 as *mut _)));
                }
                LRESULT(1)
            }
            WM_SETCURSOR
                if CAPTURING.load(Ordering::Relaxed)
                    && (lp.0 & 0xffff) as u32 == HTCLIENT
                    && (wp.0 == hwnd.0 as usize
                        || IsChild(hwnd, HWND(wp.0 as *mut _)).as_bool()) =>
            {
                SetCursor(None);
                LRESULT(1)
            }
            WM_PAINT => {
                if CAPTURING.load(Ordering::Relaxed) {
                    REPAINT_PENDING.store(true, Ordering::Relaxed);
                }
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                if !CAPTURING.load(Ordering::Relaxed) {
                    crate::presentation::with_window(hwnd, |ui| ui.paint(dc));
                }
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}

unsafe extern "system" fn enumerate(hwnd: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: parameter points to the Vec owned by choose_source during synchronous EnumWindows.
    unsafe {
        if IsWindowVisible(hwnd).as_bool()
            && GetWindow(hwnd, GW_OWNER)
                .map(|owner| owner.is_invalid())
                .unwrap_or(true)
        {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
                return BOOL(1);
            }
            let mut title = [0u16; 512];
            let count = GetWindowTextW(hwnd, &mut title);
            if count > 0 {
                let title = String::from_utf16_lossy(&title[..count as usize]);
                (*(parameter.0 as *mut Vec<(HWND, String)>)).push((hwnd, title));
            }
        }
    }
    BOOL(1)
}

fn choose_source(owner: HWND) -> Result<Option<HWND>> {
    let mut windows = Vec::<(HWND, String)>::new();
    // SAFETY: enumeration writes only into this live Vec; popup is destroyed after synchronous selection.
    unsafe {
        EnumWindows(Some(enumerate), LPARAM(&mut windows as *mut _ as isize))?;
        let menu = CreatePopupMenu()?;
        for (index, (_, title)) in windows.iter().enumerate() {
            let title = wide(&title.replace('&', "&&"));
            AppendMenuW(menu, MF_STRING, index + 100, PCWSTR(title.as_ptr()))?;
        }
        let mut position = POINT::default();
        GetCursorPos(&mut position)?;
        let _ = SetForegroundWindow(owner);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY,
            position.x,
            position.y,
            None,
            owner,
            None,
        )
        .0 as usize;
        DestroyMenu(menu)?;
        Ok(selected
            .checked_sub(100)
            .and_then(|index| windows.get(index))
            .map(|item| item.0))
    }
}

fn replace_hotkeys(
    hwnd: HWND,
    new: &Settings,
    old: &Settings,
    actual: &mut [cursorcue_config::Hotkey; 5],
) -> Result<()> {
    // SAFETY: shortcut IDs belong to this HWND. On any failure, all newly registered IDs are removed before restoring old bindings.
    unsafe {
        for id in 1..=5 {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
        *actual = [cursorcue_config::Hotkey {
            modifiers: 0,
            key: 0,
        }; 5];
        for (index, key) in new.hotkeys.iter().enumerate() {
            if key.key == 0 {
                continue;
            }
            if RegisterHotKey(
                Some(hwnd),
                index as i32 + 1,
                HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
                key.key,
            )
            .is_err()
            {
                for id in 1..=5 {
                    let _ = UnregisterHotKey(Some(hwnd), id);
                }
                *actual = [cursorcue_config::Hotkey {
                    modifiers: 0,
                    key: 0,
                }; 5];
                let mut restored = true;
                for (index, key) in old.hotkeys.iter().enumerate() {
                    if key.key != 0 {
                        if RegisterHotKey(
                            Some(hwnd),
                            index as i32 + 1,
                            HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
                            key.key,
                        )
                        .is_err()
                        {
                            restored = false;
                        } else {
                            actual[index] = *key;
                        }
                    }
                }
                if !restored {
                    for id in 1..=5 {
                        let _ = UnregisterHotKey(Some(hwnd), id);
                    }
                    *actual = [cursorcue_config::Hotkey {
                        modifiers: 0,
                        key: 0,
                    }; 5];
                }
                return Err(windows::core::Error::new(
                    E_FAIL,
                    if restored {
                        "This shortcut is already being used by another application or Windows. Your previous shortcuts were restored."
                    } else {
                        "This shortcut is already being used by another application or Windows. Previous shortcuts are unavailable, so all global shortcuts were disabled. Use the Tools menu and change them in Settings."
                    },
                ));
            }
            actual[index] = *key;
        }
        Ok(())
    }
}

struct App {
    hwnd: HWND,
    capture: Option<Capture>,
    renderer: Option<Renderer>,
    surface: Option<HWND>,
    source: Option<HWND>,
    cursor: Cursor,
    last_tick: Instant,
    frames: u64,
    settings: Settings,
    config_path: PathBuf,
    settings_ui: Option<Box<SettingsWindow>>,
    recovery_used: bool,
    registered_hotkeys: [cursorcue_config::Hotkey; 5],
}
fn check_first_frame(frames: u64, started: Instant, now: Instant) -> Result<()> {
    if frames == 0 && now.duration_since(started) > Duration::from_secs(5) {
        return Err(windows::core::Error::new(
            E_FAIL,
            "Windows did not provide a capture frame. Keep the source window visible or choose another window.",
        ));
    }
    Ok(())
}
impl App {
    fn configure_motion(&mut self) {
        let rate = match self.settings.smoothing {
            Smoothing::Off => 0.0,
            Smoothing::Light => 32.0,
            Smoothing::Medium => 18.0,
            Smoothing::Strong => 10.0,
        };
        self.cursor
            .set_motion(rate, self.settings.animation_duration_ms as f32 / 1000.0);
    }
    fn physical_sample(&self) -> Result<Option<Point>> {
        let (Some(source), Some(renderer)) = (self.source, self.renderer.as_ref()) else {
            return Ok(None);
        };
        // SAFETY: source HWND is validated by Windows on each mapping call. Outputs are writable stack values.
        unsafe {
            let mut bounds = RECT::default();
            DwmGetWindowAttribute(
                source,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut bounds as *mut _ as *mut _,
                size_of::<RECT>() as u32,
            )?;
            let mut position = POINT::default();
            GetCursorPos(&mut position)?;
            let size = renderer.source_size();
            Ok(map_to_source(
                Point {
                    x: position.x as f32,
                    y: position.y as f32,
                },
                [
                    bounds.left as f32,
                    bounds.top as f32,
                    bounds.right as f32,
                    bounds.bottom as f32,
                ],
                Point {
                    x: size.0 as f32,
                    y: size.1 as f32,
                },
            ))
        }
    }
    fn open_settings(&mut self) -> Result<()> {
        if let Some(form) = &self.settings_ui {
            // SAFETY: this form HWND was created on this thread; Windows checks whether it remains live.
            unsafe {
                if IsWindow(Some(form.hwnd)).as_bool() {
                    form.show();
                    return Ok(());
                }
            }
        }
        self.settings_ui = None;
        let form = SettingsWindow::new(Some(self.hwnd), &self.settings)?;
        form.set_status(&shortcut_status(&self.settings, &self.registered_hotkeys));
        form.show();
        self.settings_ui = Some(form);
        Ok(())
    }
    fn save_settings(&mut self) -> Result<()> {
        let Some(form) = self.settings_ui.as_ref() else {
            return Ok(());
        };
        let candidate = match form.read() {
            Ok(candidate) => candidate,
            Err(error) => {
                form.set_status(&error.message());
                return Ok(());
            }
        };
        let previous = self.registered_hotkeys;
        let mut previous_settings = self.settings.clone();
        previous_settings.hotkeys = previous;
        let shortcuts_changed = candidate.hotkeys != self.settings.hotkeys;
        if shortcuts_changed
            && let Err(error) = replace_hotkeys(
                self.hwnd,
                &candidate,
                &previous_settings,
                &mut self.registered_hotkeys,
            )
        {
            form.set_status(&format!(
                "{} Choose another key/modifier or clear the key to disable it.",
                error.message()
            ));
            return Ok(());
        }
        if let Err(error) = candidate.save(&self.config_path) {
            let empty = Settings {
                hotkeys: [cursorcue_config::Hotkey {
                    modifiers: 0,
                    key: 0,
                }; 5],
                ..Settings::default()
            };
            if shortcuts_changed
                && replace_hotkeys(
                    self.hwnd,
                    &previous_settings,
                    &empty,
                    &mut self.registered_hotkeys,
                )
                .is_err()
            {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    format!(
                        "Settings were not saved: {error}. Previous shortcuts are unavailable; global shortcuts were disabled. Use the Tools menu."
                    ),
                ));
            }
            return Err(windows::core::Error::new(E_FAIL, error.to_string()));
        }
        self.settings = candidate;
        self.configure_motion();
        if let Some(form) = &self.settings_ui {
            form.set_status(&format!(
                "Saved. {}",
                shortcut_status(&self.settings, &self.registered_hotkeys)
            ));
        }
        Ok(())
    }
    fn recover(&mut self, error: windows::core::Error) -> Result<()> {
        if !self.recovery_used
            && [
                DXGI_ERROR_DEVICE_REMOVED,
                DXGI_ERROR_DEVICE_RESET,
                DXGI_ERROR_DEVICE_HUNG,
            ]
            .contains(&error.code())
            && let Some(source) = self.source
        {
            let cursor = std::mem::take(&mut self.cursor);
            let result = self.start(source);
            self.recovery_used = true;
            if result.is_ok() {
                self.cursor = cursor;
            }
            return result;
        }
        self.stop();
        Err(error)
    }
    fn stop(&mut self) {
        self.capture = None;
        self.renderer = None;
        if let Some(surface) = self.surface.take() {
            // SAFETY: this app owns the child HWND and releases its renderer before destroying it.
            unsafe {
                let _ = DestroyWindow(surface);
            }
        }
        CAPTURING.store(false, Ordering::Relaxed);
        crate::presentation::with_window(self.hwnd, |ui| ui.set_live(false));
        // SAFETY: app HWND remains alive until after App is dropped.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), 1);
            let _ = SetWindowTextW(self.hwnd, w!("CursorCue Share"));
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }
    fn start(&mut self, source: HWND) -> Result<()> {
        let result = self.start_inner(source);
        if result.is_err() {
            self.stop();
        }
        result
    }
    fn start_inner(&mut self, source: HWND) -> Result<()> {
        self.stop();
        // SAFETY: source and output HWNDs are live; rect outputs point to writable stack values.
        unsafe {
            if !IsWindow(Some(source)).as_bool() {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "The source window has closed. Choose another window.",
                ));
            }
            let mut rect = RECT::default();
            GetClientRect(self.hwnd, &mut rect)?;
            let surface = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                Some(self.hwnd),
                None,
                None,
                None,
            )?;
            self.surface = Some(surface);
            if std::env::args().any(|arg| arg == "--diagnostic") {
                println!("Diagnostic stage: creating D3D11 renderer");
            }
            let renderer = Renderer::new(
                surface,
                (rect.right - rect.left).max(1) as u32,
                (rect.bottom - rect.top).max(1) as u32,
            )?;
            if std::env::args().any(|arg| arg == "--diagnostic") {
                println!("Diagnostic stage: creating WGC capture session");
            }
            let capture = Capture::new(&renderer.device, source)?;
            if !capture.cursor_excluded()? {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Windows could not exclude the physical cursor from capture.",
                ));
            }
            self.capture = Some(capture);
            self.renderer = Some(renderer);
            self.source = Some(source);
            self.cursor = Cursor::default();
            self.configure_motion();
            self.recovery_used = false;
            self.frames = 0;
            self.last_tick = Instant::now();
            CAPTURING.store(true, Ordering::Relaxed);
            crate::presentation::with_window(self.hwnd, |ui| ui.set_live(true));
            if SetTimer(Some(self.hwnd), 1, 16, None) == 0 {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Windows could not start the sharing timer.",
                ));
            }
            let _ = ShowWindow(
                self.hwnd,
                if std::env::args().any(|arg| arg == "--diagnostic") {
                    SW_SHOWNOACTIVATE
                } else {
                    SW_SHOW
                },
            );
            SetWindowTextW(
                self.hwnd,
                w!("CursorCue Share - Following your mouse - Share this window"),
            )?;
        }
        Ok(())
    }
    fn draw(&mut self) -> Result<()> {
        let Some(source) = self.source else {
            return Ok(());
        };
        if self.capture.is_none() {
            return Ok(());
        }
        // SAFETY: handle liveness is checked each tick; stack rect and position pointers are valid.
        unsafe {
            if !IsWindow(Some(source)).as_bool() {
                self.stop();
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "The window you were sharing has closed. Choose another source window.",
                ));
            }
            if IsIconic(source).as_bool() || IsIconic(self.hwnd).as_bool() {
                return Ok(());
            }
            let Some(renderer) = &mut self.renderer else {
                return Ok(());
            };
            let Some(capture) = &mut self.capture else {
                return Ok(());
            };
            if let Some(frame) = capture.latest()? {
                renderer.update_source(&frame.texture)?;
                drop(frame);
                self.frames += 1;
            }
            check_first_frame(self.frames, self.last_tick, Instant::now())?;
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            if let Some(surface) = self.surface
                && renderer.output_size()
                    != (
                        (client.right - client.left).max(1) as u32,
                        (client.bottom - client.top).max(1) as u32,
                    )
            {
                SetWindowPos(
                    surface,
                    None,
                    0,
                    0,
                    client.right - client.left,
                    client.bottom - client.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
            renderer.resize(
                (client.right - client.left).max(1) as u32,
                (client.bottom - client.top).max(1) as u32,
            )?;
            let mut bounds = RECT::default();
            DwmGetWindowAttribute(
                source,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut bounds as *mut _ as *mut _,
                size_of::<RECT>() as u32,
            )?;
            let mut physical = POINT::default();
            GetCursorPos(&mut physical)?;
            let size = renderer.source_size();
            renderer.configure_cursor(
                self.settings.cursor_scale * GetDpiForWindow(source).max(96) as f32 / 96.0,
                self.settings.opacity,
                match self.settings.cursor_style {
                    CursorStyle::Arrow => 0,
                    CursorStyle::Dot => 1,
                    CursorStyle::Circle => 2,
                },
            );
            let mapped = map_to_source(
                Point {
                    x: physical.x as f32,
                    y: physical.y as f32,
                },
                [
                    bounds.left as f32,
                    bounds.top as f32,
                    bounds.right as f32,
                    bounds.bottom as f32,
                ],
                Point {
                    x: size.0 as f32,
                    y: size.1 as f32,
                },
            );
            let now = Instant::now();
            self.cursor
                .track(mapped, (now - self.last_tick).as_secs_f32());
            if self.frames > 0 {
                self.last_tick = now;
            }
            if REPAINT_PENDING.swap(false, Ordering::Relaxed) {
                renderer.invalidate();
            }
            renderer.render(&self.cursor)?;
        }
        Ok(())
    }
    fn dispatch(&mut self, id: u32) -> Result<()> {
        match id {
            1 => self.cursor.freeze(),
            2 => self.cursor.hide(),
            3 => self.cursor.resume(self.settings.animation_enabled),
            4 => {
                let physical = self.physical_sample()?;
                self.cursor.track(physical, 0.0);
                self.cursor.drop_here();
            }
            5 => {
                if self.capture.is_some() {
                    self.stop();
                } else if let Some(source) = self.source {
                    self.start(source)?;
                } else {
                    self.dispatch(CHOOSE)?;
                }
            }
            CHOOSE => {
                if let Some(source) = choose_source(self.hwnd)? {
                    self.start(source)?;
                }
            }
            QUIT => {
                self.stop();
                // SAFETY: shutdown is requested for the current thread's message loop.
                unsafe {
                    PostQuitMessage(0);
                }
            }
            TRAY => self.tray_menu()?,
            SETTINGS => self.open_settings()?,
            HELP => show_guide(self.hwnd),
            SAVE_SETTINGS => self.save_settings()?,
            RESET_SETTINGS => {
                if let Some(form) = &self.settings_ui {
                    form.populate(&Settings::default())?;
                }
            }
            STOP => self.stop(),
            _ => {}
        }
        if (1..=5).contains(&id) && self.capture.is_some() {
            let state = match self.cursor.mode {
                cursorcue_core::Mode::Frozen => "Shared cursor frozen",
                cursorcue_core::Mode::Hidden => "Shared cursor hidden",
                _ => "Following your mouse",
            };
            let title = wide(&format!("CursorCue Share - {state} - Share this window"));
            unsafe {
                SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()))?;
            }
        }
        Ok(())
    }
    fn tray_menu(&mut self) -> Result<()> {
        // SAFETY: popup lifetime is scoped to tracking; owner HWND belongs to this thread.
        unsafe {
            let menu = CreatePopupMenu()?;
            for (id, label) in [
                (
                    5,
                    if self.capture.is_some() {
                        "CursorCue: On"
                    } else {
                        "CursorCue: Off"
                    },
                ),
                (CHOOSE, "Choose a window to share"),
                (1, "Freeze cursor"),
                (2, "Hide / reveal cursor"),
                (3, "Resume cursor"),
                (4, "Drop cursor here"),
                (SETTINGS, "Cursor size && shortcuts..."),
                (HELP, "How to use CursorCue..."),
                (QUIT, "Quit CursorCue"),
            ] {
                let label = wide(label);
                AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(label.as_ptr()))?;
            }
            let mut point = POINT::default();
            GetCursorPos(&mut point)?;
            let _ = SetForegroundWindow(self.hwnd);
            let id = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as u32;
            DestroyMenu(menu)?;
            self.dispatch(id)
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "system" fn fixture_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    // SAFETY: the fixture owns this HWND on its independently pumping source thread.
    unsafe {
        match message {
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                FillRect(dc, &rect, HBRUSH(GetStockObject(WHITE_BRUSH).0));
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
struct DiagnosticFixture {
    hwnd: HWND,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl DiagnosticFixture {
    fn new() -> Result<Self> {
        let (sender, receiver) = sync_channel::<std::result::Result<usize, String>>(1);
        let thread = std::thread::spawn(move || {
            // SAFETY: the source window is created, pumped and destroyed on this one source thread.
            let created: Result<HWND> = unsafe {
                (|| {
                    let instance = GetModuleHandleW(None)?;
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(fixture_proc),
                        hInstance: instance.into(),
                        hCursor: LoadCursorW(None, IDC_ARROW)?,
                        hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
                        lpszClassName: w!("CursorCueDiagnosticFixture"),
                        ..Default::default()
                    };
                    if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                        return Err(windows::core::Error::from_thread());
                    }
                    let hwnd = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        class.lpszClassName,
                        w!("CursorCue diagnostic source"),
                        WS_OVERLAPPEDWINDOW,
                        20,
                        20,
                        640,
                        480,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    )?;
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    Ok(hwnd)
                })()
            };
            match created {
                Ok(hwnd) => {
                    let _ = sender.send(Ok(hwnd.0 as usize));
                    let mut message = MSG::default();
                    // SAFETY: this loop dispatches only the source thread's own window messages.
                    unsafe {
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        });
        let hwnd = receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?
            .map_err(|error| windows::core::Error::new(E_FAIL, error))?;
        Ok(Self {
            hwnd: HWND(hwnd as *mut _),
            thread: Some(thread),
        })
    }
    fn close(&self) -> Result<()> {
        if self
            .thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
        {
            return Ok(());
        }
        // SAFETY: asynchronous close is posted to the fixture's owning thread rather than destroying a foreign HWND.
        unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
    }
}
impl Drop for DiagnosticFixture {
    fn drop(&mut self) {
        let _ = self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct NativeResources {
    hwnd: HWND,
    hotkeys: Vec<i32>,
    tray: Option<NOTIFYICONDATAW>,
    fixture: Option<DiagnosticFixture>,
    welcome: Option<Box<crate::presentation::Welcome>>,
}
impl Drop for NativeResources {
    fn drop(&mut self) {
        // SAFETY: these resources are owned by this thread. App is dropped first so its capture sessions are already released.
        unsafe {
            for id in &self.hotkeys {
                let _ = UnregisterHotKey(Some(self.hwnd), *id);
            }
            if let Some(tray) = &self.tray {
                let _ = Shell_NotifyIconW(NIM_DELETE, tray);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard is created only after successful RoInitialize and is dropped on the same thread.
        unsafe {
            RoUninitialize();
        }
    }
}

pub fn run() -> Result<()> {
    // SAFETY: window/device operations run on the owning thread; WinRT apartment lives until shutdown.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let diagnostic = std::env::args().any(|arg| arg == "--diagnostic");
        let welcome_diagnostic = diagnostic && std::env::args().any(|arg| arg == "--welcome");
        if !diagnostic && let Ok(existing) = FindWindowW(w!("CursorCuePresentationClass"), None) {
            let _ = ShowWindow(existing, SW_RESTORE);
            let _ = SetForegroundWindow(existing);
            let _ = PostMessageW(
                Some(existing),
                WM_COMMAND,
                WPARAM(SETTINGS as usize),
                LPARAM(0),
            );
            return Ok(());
        }
        let _instance = if diagnostic {
            None
        } else {
            match InstanceGuard::acquire("Local\\CursorCue.Desktop")? {
                Some(guard) => Some(guard),
                None => return Ok(()),
            }
        };
        let config_path = if diagnostic {
            std::env::temp_dir().join(format!("CursorCue-diagnostic-{}.json", std::process::id()))
        } else {
            appdata_config_path()
                .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?
        };
        let loaded = if diagnostic {
            cursorcue_config::LoadResult {
                settings: Settings::default(),
                warning: None,
            }
        } else {
            Settings::load(&config_path)
                .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?
        };
        if let Some(warning) = loaded.warning {
            report_error(&warning);
        }
        RoInitialize(RO_INIT_MULTITHREADED)?;
        let _apartment = Apartment;
        if !windows::Graphics::Capture::GraphicsCaptureSession::IsSupported()? {
            return Err(windows::core::Error::new(
                E_FAIL,
                "Windows Graphics Capture is unavailable. CursorCue requires Windows 11 or Windows 10 version 2004 or newer.",
            ));
        }
        let (sender, receiver) = sync_channel(32);
        let _ = COMMANDS.set(sender);
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance(1)),
            )?,
            lpszClassName: w!("CursorCuePresentationClass"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let menu = CreateMenu()?;
        let actions = CreatePopupMenu()?;
        for (id, label) in [
            (CHOOSE, "Choose window..."),
            (1, "Freeze"),
            (2, "Hide / reveal"),
            (3, "Resume"),
            (4, "Drop"),
            (5, "Toggle CursorCue"),
            (SETTINGS, "Cursor size && shortcuts..."),
            (HELP, "How to use CursorCue..."),
            (QUIT, "Quit CursorCue"),
        ] {
            let label = wide(label);
            AppendMenuW(actions, MF_STRING, id as usize, PCWSTR(label.as_ptr()))?;
        }
        AppendMenuW(menu, MF_POPUP, actions.0 as usize, w!("&Tools"))?;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.lpszClassName,
            w!("CursorCue Share"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VSCROLL,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            650,
            None,
            Some(menu),
            Some(instance.into()),
            None,
        )?;
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let work = monitor.rcWork;
            let width = (1000 * dpi / 96).min((work.right - work.left - 32).max(360));
            let height = (650 * dpi / 96).min((work.bottom - work.top - 32).max(280));
            SetWindowPos(
                hwnd,
                None,
                work.left + (work.right - work.left - width) / 2,
                work.top + (work.bottom - work.top - height) / 2,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
        }
        let mut resources = NativeResources {
            hwnd,
            hotkeys: (1..=5).collect(),
            tray: None,
            fixture: None,
            welcome: None,
        };
        resources.welcome = Some(crate::presentation::Welcome::new(hwnd)?);
        let _ = ShowWindow(
            hwnd,
            if std::env::args().any(|arg| arg == "--diagnostic") {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOW
            },
        );

        let mut registered_hotkeys = [cursorcue_config::Hotkey {
            modifiers: 0,
            key: 0,
        }; 5];
        for (index, key) in loaded.settings.hotkeys.iter().enumerate() {
            if key.key == 0 || diagnostic {
                continue;
            }
            if let Ok(()) = RegisterHotKey(
                Some(hwnd),
                index as i32 + 1,
                HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
                key.key,
            ) {
                registered_hotkeys[index] = *key;
            }
        }
        let mut tray = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_MESSAGE,
            hIcon: LoadIconW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance(1)),
            )?,
            ..Default::default()
        };
        let tip = wide("CursorCue - screen-sharing cursor control");
        tray.szTip[..tip.len()].copy_from_slice(&tip);
        if !Shell_NotifyIconW(NIM_ADD, &tray).as_bool() {
            report_error(
                "Windows could not add the CursorCue tray icon. Controls remain available in the Tools menu.",
            );
        } else {
            resources.tray = Some(tray);
        }
        let mut app = App {
            hwnd,
            capture: None,
            renderer: None,
            surface: None,
            source: None,
            cursor: Cursor::default(),
            last_tick: Instant::now(),
            frames: 0,
            settings: loaded.settings,
            config_path,
            settings_ui: None,
            recovery_used: false,
            registered_hotkeys,
        };
        if !diagnostic && app.settings.hotkeys != app.registered_hotkeys {
            app.open_settings()?;
        }
        let diagnostic_seconds = std::env::args()
            .find_map(|arg| {
                arg.strip_prefix("--seconds=")
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .unwrap_or(10);
        let started = Instant::now();
        if diagnostic && !welcome_diagnostic {
            let fixture = DiagnosticFixture::new()?;
            let source = fixture.hwnd;
            resources.fixture = Some(fixture);
            app.start(source)?;
            if std::env::args().any(|arg| arg == "--settings") {
                app.open_settings()?;
            }
            println!(
                "CursorCue diagnostic: cursor capture excluded = true; native D3D11 presentation started"
            );
        }
        if welcome_diagnostic && SetTimer(Some(hwnd), 1, 1000, None) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let mut message = MSG::default();
        let mut pixels_verified = false;
        let mut resized = false;
        let mut restarted = false;
        let mut closing_source = false;
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                report_error("The Windows message loop failed.");
                break;
            }
            if result == 0 {
                break;
            }
            let dialog_handled = app.settings_ui.as_ref().is_some_and(|form| {
                IsWindow(Some(form.hwnd)).as_bool()
                    && IsDialogMessageW(form.hwnd, &message).as_bool()
            }) || (!CAPTURING.load(Ordering::Relaxed)
                && (message.hwnd == hwnd || IsChild(hwnd, message.hwnd).as_bool())
                && IsDialogMessageW(hwnd, &message).as_bool());
            if !dialog_handled {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if message.message == WM_KEYDOWN
                && message.wParam.0 == VK_TAB.0 as usize
                && let Some(form) = &app.settings_ui
            {
                form.reveal_focus();
            }
            if message.message == WM_KEYDOWN && message.wParam.0 == VK_TAB.0 as usize {
                crate::presentation::with_window(hwnd, |ui| ui.reveal_focus());
            }
            for id in receiver.try_iter() {
                if let Err(error) = app.dispatch(id) {
                    if ![SETTINGS, SAVE_SETTINGS, RESET_SETTINGS].contains(&id) {
                        app.stop();
                    }
                    if diagnostic {
                        return Err(error);
                    }
                    if [SETTINGS, SAVE_SETTINGS, RESET_SETTINGS].contains(&id) {
                        report_error(&format!("Settings could not be applied.\n{error}"));
                    } else {
                        report_error(&format!(
                            "Screen sharing stopped safely.\n{error}\nChoose a window to try again."
                        ));
                    }
                }
            }
            if INPUT_OVERFLOW.swap(false, Ordering::Relaxed) {
                report_error("Too many controls arrived at once. Please repeat the last shortcut.");
            }
            if DRAW_PENDING.swap(false, Ordering::Relaxed)
                && let Err(error) = app.draw()
            {
                if diagnostic {
                    if closing_source
                        && app
                            .source
                            .is_some_and(|source| !IsWindow(Some(source)).as_bool())
                        && app.capture.is_none()
                        && app.renderer.is_none()
                        && app.surface.is_none()
                    {
                        println!(
                            "CursorCue diagnostic: source-close released capture, GPU renderer and native surface safely"
                        );
                        break;
                    }
                    return Err(error);
                }
                if let Err(error) = app.recover(error) {
                    report_error(&format!(
                        "Screen sharing stopped safely.\n{error}\nChoose a source to try again."
                    ));
                }
            }
            if diagnostic {
                if !resized
                    && started.elapsed() >= Duration::from_secs(2)
                    && let Some(source) = app.source
                {
                    SetWindowPos(
                        source,
                        None,
                        20,
                        20,
                        1920,
                        1080,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )?;
                    pixels_verified = false;
                    resized = true;
                    println!("CursorCue diagnostic: source resize to1080p requested");
                }
                if !restarted
                    && started.elapsed() >= Duration::from_secs(4)
                    && let Some(source) = app.source
                {
                    app.stop();
                    app.start(source)?;
                    pixels_verified = false;
                    restarted = true;
                    println!("CursorCue diagnostic: stop/start lifecycle completed");
                }
                if !pixels_verified
                    && app.frames > 0
                    && let Some(renderer) = app.renderer.as_mut()
                {
                    let size = renderer.source_size();
                    let origin = Point {
                        x: size.0 as f32 * 0.5,
                        y: size.1 as f32 * 0.5,
                    };
                    let point = Point {
                        x: origin.x,
                        y: origin.y + 15.0,
                    };
                    let mut probe = Cursor::default();
                    probe.hide();
                    let hidden = renderer.diagnostic_pixel(&probe, point)?;
                    probe.track(Some(origin), 0.016);
                    probe.drop_here();
                    let shown = renderer.diagnostic_pixel(&probe, point)?;
                    if hidden[..3].iter().any(|value| *value < 240)
                        || shown[..3].iter().any(|value| *value > 80)
                    {
                        return Err(windows::core::Error::new(
                            E_FAIL,
                            format!(
                                "GPU composition pixel validation failed: hidden={hidden:?}, visible={shown:?}"
                            ),
                        ));
                    }
                    println!(
                        "CursorCue diagnostic: GPU output pixel verified; source={hidden:?}, synthetic arrow={shown:?}"
                    );
                    for scale in [1.0, 6.0] {
                        renderer.configure_cursor(scale, 1.0, 1);
                        let dot = renderer.diagnostic_pixel(&probe, origin)?;
                        if !(240..=245).contains(&dot[2]) {
                            return Err(windows::core::Error::new(
                                E_FAIL,
                                format!("Dot hotspot pixel failed: {dot:?}"),
                            ));
                        }
                        renderer.configure_cursor(scale, 1.0, 2);
                        let center = renderer.diagnostic_pixel(&probe, origin)?;
                        let rim = renderer.diagnostic_pixel(
                            &probe,
                            Point {
                                x: origin.x + 12.0 * scale,
                                y: origin.y,
                            },
                        )?;
                        if center[..3].iter().any(|value| *value < 240)
                            || rim[..3].iter().any(|value| *value > 80)
                        {
                            return Err(windows::core::Error::new(
                                E_FAIL,
                                format!(
                                    "Circle hotspot/size pixel failed: center={center:?},rim={rim:?}"
                                ),
                            ));
                        }
                    }
                    renderer.configure_cursor(1.0, 0.5, 0);
                    let faded = renderer.diagnostic_pixel(&probe, point)?;
                    if faded[..3].iter().any(|value| !(100..=190).contains(value)) {
                        return Err(windows::core::Error::new(
                            E_FAIL,
                            format!("Cursor opacity pixel failed: {faded:?}"),
                        ));
                    }
                    renderer.configure_cursor(1.0, 1.0, 0);
                    println!(
                        "CursorCue diagnostic: Arrow/Dot/Circle hotspot,600% physical size and50% opacity GPU pixels verified"
                    );
                    pixels_verified = true;
                }
                if let Some(source) = app.source {
                    let caption = wide(&format!(
                        "CursorCue diagnostic source - {}",
                        started.elapsed().as_millis() / 500
                    ));
                    let _ = SetWindowTextW(source, PCWSTR(caption.as_ptr()));
                }
            }
            if welcome_diagnostic && started.elapsed() >= Duration::from_secs(diagnostic_seconds) {
                println!("CursorCue welcome UI diagnostic complete.");
                PostQuitMessage(0);
                continue;
            }
            if diagnostic
                && !welcome_diagnostic
                && !closing_source
                && started.elapsed() >= Duration::from_secs(diagnostic_seconds)
            {
                println!(
                    "CursorCue diagnostic: {} frames composited in {:.2}s ({} arrival events)",
                    app.frames,
                    started.elapsed().as_secs_f32(),
                    app.capture
                        .as_ref()
                        .map(Capture::frames_arrived)
                        .unwrap_or_default()
                );
                if app.frames == 0 || !pixels_verified {
                    return Err(windows::core::Error::new(
                        E_FAIL,
                        "Diagnostic received no captured frames.",
                    ));
                }
                if let Some(fixture) = resources.fixture.as_ref() {
                    println!(
                        "Diagnostic teardown: posting source close to independent owning thread"
                    );
                    fixture.close()?;
                    closing_source = true;
                }
            }
            if closing_source && started.elapsed() > Duration::from_secs(diagnostic_seconds + 5) {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Source-close diagnostic timed out.",
                ));
            }
        }
        drop(app);
        drop(resources);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cursorcue_config::Hotkey;
    #[test]
    fn welcome_mouse_wheel_keeps_instruction_positions_fixed() {
        let window = Window::new();
        let _welcome = crate::presentation::Welcome::new(window.0).unwrap();
        unsafe {
            let label = GetDlgItem(Some(window.0), 102).unwrap();
            let mut before = RECT::default();
            GetWindowRect(label, &mut before).unwrap();
            window_proc(
                window.0,
                WM_MOUSEWHEEL,
                WPARAM((-120i16 as u16 as usize) << 16),
                LPARAM(0),
            );
            let mut after = RECT::default();
            GetWindowRect(label, &mut after).unwrap();
            assert_eq!(
                before.top, after.top,
                "welcome instructions must not move with the wheel"
            );
        }
    }
    #[test]
    fn welcome_wheel_does_not_repaint_or_shift_text_when_content_fits() {
        let window = Window::new();
        unsafe {
            SetWindowPos(window.0, None, 0, 0, 1000, 900, SWP_NOZORDER | SWP_NOMOVE).unwrap();
        }
        let _welcome = crate::presentation::Welcome::new(window.0).unwrap();
        unsafe {
            let _ = ValidateRect(Some(window.0), None);
        }
        unsafe {
            window_proc(
                window.0,
                WM_MOUSEWHEEL,
                WPARAM((-120i16 as u16 as usize) << 16),
                LPARAM(0),
            );
        }
        unsafe {
            assert!(
                !GetUpdateRect(window.0, None, false).as_bool(),
                "wheel input must not repaint a stationary setup page"
            );
        }
    }
    #[test]
    fn welcome_erase_repaints_background_for_themed_children() {
        let window = Window::new();
        let _welcome = crate::presentation::Welcome::new(window.0).unwrap();
        unsafe {
            let screen = GetDC(Some(window.0));
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, 64, 64);
            let previous = SelectObject(dc, bitmap.into());
            let stale = CreateSolidBrush(COLORREF(0xff00ff));
            FillRect(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 64,
                    bottom: 64,
                },
                stale,
            );
            assert_eq!(
                window_proc(window.0, WM_ERASEBKGND, WPARAM(dc.0 as usize), LPARAM(0)).0,
                1
            );
            let color = GetPixel(dc, 20, 20);
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteObject(stale.into());
            let _ = DeleteDC(dc);
            ReleaseDC(Some(window.0), screen);
            assert_eq!(
                color,
                COLORREF(0xfbf6f4),
                "parent must erase old button pixels"
            );
        }
    }
    #[test]
    fn presentation_has_direct_choose_settings_and_help_controls() {
        let window = Window::new();
        let _welcome = crate::presentation::Welcome::new(window.0).unwrap();
        unsafe {
            let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
            for id in [CHOOSE, SETTINGS, HELP] {
                let button =
                    GetDlgItem(Some(window.0), id as i32).expect("direct presentation control");
                assert!(IsWindowVisible(button).as_bool());
                assert!(IsWindowEnabled(button).as_bool());
            }
        }
    }
    #[test]
    fn presentation_controls_hide_for_capture_and_restore_with_dpi_and_scrolling() {
        let window = Window::new();
        let welcome = crate::presentation::Welcome::new(window.0).unwrap();
        unsafe {
            let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
            let button = GetDlgItem(Some(window.0), CHOOSE as i32).unwrap();
            welcome.set_live(true);
            assert!(!IsWindowVisible(button).as_bool());
            welcome.update_dpi(192).unwrap();
            welcome.set_live(false);
            assert!(IsWindowVisible(button).as_bool());
            welcome.scroll(SB_BOTTOM.0);
            let footer = GetDlgItem(Some(window.0), 108).unwrap();
            let mut bounds = RECT::default();
            GetWindowRect(footer, &mut bounds).unwrap();
            let mut point = POINT {
                x: bounds.left,
                y: bounds.bottom,
            };
            ScreenToClient(window.0, &mut point).ok().unwrap();
            let mut client = RECT::default();
            GetClientRect(window.0, &mut client).unwrap();
            assert!(
                point.y <= client.bottom && point.y > 0,
                "guide footer must remain reachable"
            );
            let _ = SetFocus(Some(button));
            welcome.reveal_focus();
            GetWindowRect(button, &mut bounds).unwrap();
            point = POINT {
                x: bounds.left,
                y: bounds.top,
            };
            ScreenToClient(window.0, &mut point).ok().unwrap();
            assert!(point.y >= 0, "keyboard focus must reveal the first action");
        }
    }
    #[test]
    fn single_instance_guard_rejects_duplicates_and_releases_on_exit() {
        let name = format!("Local\\CursorCue.Test.{}", std::process::id());
        let first = InstanceGuard::acquire(&name).unwrap().unwrap();
        assert!(InstanceGuard::acquire(&name).unwrap().is_none());
        drop(first);
        assert!(InstanceGuard::acquire(&name).unwrap().is_some());
    }
    #[test]
    fn gpu_cursor_keeps_its_head_and_tail_visible_at_source_edges() {
        let (_window, mut renderer, _texture) = gpu_scene();
        renderer.configure_cursor(1.0, 1.0, 0);
        let mut cursor = Cursor::default();
        cursor.position = Point { x: 99.0, y: 99.0 };
        for point in [Point { x: 77.0, y: 79.0 }, Point { x: 94.0, y: 97.0 }] {
            let pixel = renderer
                .diagnostic_pixel(&cursor, point)
                .expect("edge cursor probe");
            assert!(pixel[0] < 100, "cursor head/tail was clipped: {pixel:?}");
        }
    }
    fn gpu_scene() -> (
        Window,
        Renderer,
        windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    ) {
        use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};
        let window = Window::new();
        let mut renderer = Renderer::new(window.0, 120, 100).expect("GPU renderer");
        let pixels = vec![255u8; 100 * 100 * 4];
        let mut texture = None;
        unsafe {
            renderer
                .device
                .CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: 100,
                        Height: 100,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                        ..Default::default()
                    },
                    Some(&D3D11_SUBRESOURCE_DATA {
                        pSysMem: pixels.as_ptr().cast(),
                        SysMemPitch: 400,
                        SysMemSlicePitch: 0,
                    }),
                    Some(&mut texture),
                )
                .expect("source texture");
        }
        let texture = texture.expect("texture");
        renderer.update_source(&texture).expect("source copy");
        (window, renderer, texture)
    }
    #[test]
    fn gpu_skips_static_frames_but_updates_motion_visibility_source_and_size() {
        let (_window, mut renderer, texture) = gpu_scene();
        let mut cursor = Cursor::default();
        cursor.position = Point { x: 40.0, y: 40.0 };
        assert!(renderer.render(&cursor).unwrap());
        for _ in 0..120 {
            assert!(
                !renderer.render(&cursor).unwrap(),
                "static source must not redraw"
            );
        }
        cursor.freeze();
        assert!(!renderer.render(&cursor).unwrap());
        cursor.hide();
        assert!(renderer.render(&cursor).unwrap());
        cursor.position = Point { x: 70.0, y: 60.0 };
        assert!(
            !renderer.render(&cursor).unwrap(),
            "hidden motion must not redraw"
        );
        cursor.hide();
        assert!(renderer.render(&cursor).unwrap());
        cursor.position.x -= 10.0;
        assert!(renderer.render(&cursor).unwrap());
        renderer.update_source(&texture).unwrap();
        assert!(renderer.render(&cursor).unwrap());
        assert!(!renderer.render(&cursor).unwrap());
        renderer.resize(240, 200).unwrap();
        assert!(renderer.render(&cursor).unwrap());
        renderer.configure_cursor(1.5, 0.5, 2);
        assert!(renderer.render(&cursor).unwrap());
        assert!(!renderer.render(&cursor).unwrap());
        let _ = renderer
            .diagnostic_pixel(&cursor, Point { x: 50.0, y: 50.0 })
            .unwrap();
        assert!(
            renderer.render(&cursor).unwrap(),
            "diagnostic probes must invalidate presentation state"
        );
    }
    #[test]
    fn missing_first_frame_times_out_but_static_captured_content_remains_valid() {
        let started = Instant::now();
        assert!(check_first_frame(0, started, started + Duration::from_millis(4999)).is_ok());
        assert!(check_first_frame(0, started, started + Duration::from_millis(5001)).is_err());
        assert!(check_first_frame(1, started, started + Duration::from_secs(1800)).is_ok());
    }
    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            unsafe {
                Self(
                    CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("STATIC"),
                        w!("CursorCue test"),
                        WS_OVERLAPPEDWINDOW,
                        0,
                        0,
                        300,
                        200,
                        None,
                        None,
                        None,
                        None,
                    )
                    .expect("native window"),
                )
            }
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                for id in 1..=100 {
                    let _ = UnregisterHotKey(Some(self.0), id);
                }
                let _ = DestroyWindow(self.0);
            }
        }
    }
    #[test]
    fn shortcut_conflict_restores_previous_registered_binding() {
        let owner = Window::new();
        let blocker = Window::new();
        let mut old = Settings {
            hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
            ..Settings::default()
        };
        old.hotkeys[0] = Hotkey {
            modifiers: 15,
            key: 0x85,
        };
        unsafe {
            RegisterHotKey(Some(owner.0), 1, HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT, 0x85)
                .expect("previous F22 binding");
            RegisterHotKey(
                Some(blocker.0),
                99,
                HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                0x87,
            )
            .expect("reserved F24 binding");
        }
        let mut new = old.clone();
        new.hotkeys[0].key = 0x87;
        let mut actual = old.hotkeys;
        assert!(replace_hotkeys(owner.0, &new, &old, &mut actual).is_err());
        assert_eq!(actual, old.hotkeys);
        unsafe {
            assert!(
                RegisterHotKey(
                    Some(blocker.0),
                    100,
                    HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                    0x85
                )
                .is_err()
            );
            let _ = UnregisterHotKey(Some(owner.0), 1);
        }
        let empty = Settings {
            hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
            ..Settings::default()
        };
        let mut partial = empty.clone();
        partial.hotkeys[0] = Hotkey {
            modifiers: 15,
            key: 0x86,
        };
        partial.hotkeys[1] = Hotkey {
            modifiers: 15,
            key: 0x87,
        };
        actual = empty.hotkeys;
        assert!(replace_hotkeys(owner.0, &partial, &empty, &mut actual).is_err());
        assert_eq!(actual, empty.hotkeys);
        unsafe {
            RegisterHotKey(
                Some(blocker.0),
                100,
                HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                0x86,
            )
            .expect("rejected partial binding released");
        }
    }
    #[test]
    fn save_from_real_native_form_persists_and_updates_motion() {
        let owner = Window::new();
        let path =
            std::env::temp_dir().join(format!("CursorCue-native-test-{}.json", std::process::id()));
        let mut old = Settings {
            hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
            ..Settings::default()
        };
        // The unavailable key is configured but was not registered at startup.
        let blocker = Window::new();
        old.hotkeys[0] = Hotkey {
            modifiers: 15,
            key: 0x82,
        };
        unsafe {
            RegisterHotKey(
                Some(blocker.0),
                99,
                HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                0x82,
            )
            .expect("reserved shortcut");
        }
        let candidate = Settings {
            cursor_scale: 1.75,
            cursor_style: CursorStyle::Circle,
            animation_enabled: false,
            animation_duration_ms: 360,
            smoothing: Smoothing::Strong,
            ..old.clone()
        };
        let form = SettingsWindow::new(Some(owner.0), &candidate).expect("native form");
        let mut app = App {
            hwnd: owner.0,
            capture: None,
            renderer: None,
            surface: None,
            source: None,
            cursor: Cursor::default(),
            last_tick: Instant::now(),
            frames: 0,
            settings: old,
            config_path: path.clone(),
            settings_ui: Some(form),
            recovery_used: false,
            registered_hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
        };
        app.cursor.freeze();
        app.cursor.track(Some(Point { x: 800.0, y: 200.0 }), 0.01);
        app.save_settings().expect("save real form");
        assert_eq!(Settings::load(&path).expect("reload").settings, candidate);
        assert!(app.settings_ui.is_some());
        app.cursor.resume(app.settings.animation_enabled);
        assert_eq!(app.cursor.position, Point { x: 800.0, y: 200.0 });
        std::fs::remove_file(path).expect("clean task-owned config");
    }
    #[test]
    fn disk_save_failure_rolls_back_actual_bindings_without_activating_rejected_shortcuts() {
        let owner = Window::new();
        let blocker = Window::new();
        unsafe {
            RegisterHotKey(
                Some(blocker.0),
                99,
                HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                0x84,
            )
            .expect("unavailable old F21");
        }
        let mut old = Settings {
            hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
            ..Settings::default()
        };
        old.hotkeys[0] = Hotkey {
            modifiers: 15,
            key: 0x84,
        };
        let mut candidate = old.clone();
        candidate.hotkeys[0].key = 0x83;
        let path = std::env::temp_dir().join(format!(
            "CursorCue-native-save-failure-{}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("task-owned failure fixture");
        let form = SettingsWindow::new(Some(owner.0), &candidate).expect("native form");
        let mut app = App {
            hwnd: owner.0,
            capture: None,
            renderer: None,
            surface: None,
            source: None,
            cursor: Cursor::default(),
            last_tick: Instant::now(),
            frames: 0,
            settings: old.clone(),
            config_path: path.clone(),
            settings_ui: Some(form),
            recovery_used: false,
            registered_hotkeys: [Hotkey {
                modifiers: 0,
                key: 0,
            }; 5],
        };
        assert!(app.save_settings().is_err());
        assert_eq!(app.settings, old);
        assert_eq!(
            app.registered_hotkeys,
            [Hotkey {
                modifiers: 0,
                key: 0
            }; 5]
        );
        unsafe {
            RegisterHotKey(
                Some(blocker.0),
                100,
                HOT_KEY_MODIFIERS(15) | MOD_NOREPEAT,
                0x83,
            )
            .expect("rejected candidate F20 released");
        }
        std::fs::remove_dir(path).expect("clean task-owned empty directory");
    }
}
