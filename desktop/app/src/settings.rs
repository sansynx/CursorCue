use cursorcue_config::{CursorStyle, Hotkey, Settings, Smoothing};
use std::cell::Cell;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::SetScrollInfo,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            WindowsAndMessaging::*,
        },
    },
    core::{Error, PCWSTR, Result, w},
};
const MODIFIERS: [(u32, &str); 15] = [
    (6, "Ctrl + Shift"),
    (3, "Ctrl + Alt"),
    (5, "Alt + Shift"),
    (7, "Ctrl + Alt + Shift"),
    (10, "Ctrl + Win"),
    (14, "Ctrl + Win + Shift"),
    (9, "Alt + Win"),
    (1, "Alt"),
    (2, "Ctrl"),
    (4, "Shift"),
    (8, "Win"),
    (11, "Ctrl + Alt + Win"),
    (12, "Shift + Win"),
    (13, "Alt + Shift + Win"),
    (15, "Ctrl + Alt + Shift + Win"),
];
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn invalid(message: &str) -> Error {
    Error::new(E_INVALIDARG, message)
}
fn form_geometry(work: RECT, scale: f32) -> (i32, i32, i32, i32) {
    let width = ((670.0 * scale) as i32).min((work.right - work.left - 24).max(200));
    let height = ((710.0 * scale) as i32).min((work.bottom - work.top - 24).max(200));
    (
        work.left + (work.right - work.left - width) / 2,
        work.top + (work.bottom - work.top - height) / 2,
        width,
        height,
    )
}
fn scroll_to(hwnd: HWND, position: i32) {
    // SAFETY: scrollbar and child windows belong to the live form on this thread.
    unsafe {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        if GetScrollInfo(hwnd, SB_VERT, &mut info).is_err() {
            return;
        }
        let old = info.nPos;
        info.nPos = position.clamp(
            info.nMin,
            (info.nMax - info.nPage as i32 + 1).max(info.nMin),
        );
        info.fMask = SIF_POS;
        SetScrollInfo(hwnd, SB_VERT, &info, true);
        ScrollWindowEx(
            hwnd,
            0,
            old - info.nPos,
            None,
            None,
            None,
            None,
            SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
        );
    }
}
fn scroll_range(hwnd: HWND) {
    // SAFETY: GWLP_USERDATA stores only the form's content height, never a raw pointer.
    unsafe {
        let height = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as i32;
        if height <= 0 {
            return;
        }
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        let _ = GetScrollInfo(hwnd, SB_VERT, &mut info);
        let old = info.nPos;
        info.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
        info.nMin = 0;
        info.nMax = height - 1;
        info.nPage = client.bottom.max(1) as u32;
        info.nPos = old.min((height - client.bottom).max(0));
        SetScrollInfo(hwnd, SB_VERT, &info, true);
        if info.nPos != old {
            ScrollWindowEx(
                hwnd,
                0,
                old - info.nPos,
                None,
                None,
                None,
                None,
                SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
            );
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows owns this live HWND. All command dispatch is deferred to the application's message loop.
    unsafe {
        match message {
            WM_DPICHANGED => {
                let form = GetPropW(hwnd, w!("CursorCueSettings"));
                if !form.is_invalid() && lp.0 != 0 {
                    // The form is boxed for a stable address and removes this property before destruction.
                    let form = &*(form.0 as *const SettingsWindow);
                    form.update_dpi((wp.0 & 0xffff) as u32, *(lp.0 as *const RECT));
                }
                LRESULT(0)
            }
            WM_SIZE => {
                scroll_range(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_POS,
                    ..Default::default()
                };
                if GetScrollInfo(hwnd, SB_VERT, &mut info).is_ok() {
                    let form = GetPropW(hwnd, w!("CursorCueSettings"));
                    if !form.is_invalid() {
                        let form = &*(form.0 as *const SettingsWindow);
                        let delta = form.wheel_remainder.get()
                            + (wp.0 >> 16) as i16 as i32 * (90.0 * form.scale.get()) as i32;
                        form.wheel_remainder.set(delta % 120);
                        scroll_to(hwnd, info.nPos - delta / 120);
                    }
                }
                LRESULT(0)
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..Default::default()
                };
                if GetScrollInfo(hwnd, SB_VERT, &mut info).is_ok() {
                    let line = (36.0 * GetDpiForWindow(hwnd).max(96) as f32 / 96.0) as i32;
                    let target = match (wp.0 & 0xffff) as i32 {
                        value if value == SB_LINEUP.0 => info.nPos - line,
                        value if value == SB_LINEDOWN.0 => info.nPos + line,
                        value if value == SB_PAGEUP.0 => info.nPos - info.nPage as i32,
                        value if value == SB_PAGEDOWN.0 => info.nPos + info.nPage as i32,
                        value if value == SB_THUMBTRACK.0 => info.nTrackPos,
                        value if value == SB_TOP.0 => 0,
                        value if value == SB_BOTTOM.0 => info.nMax,
                        _ => info.nPos,
                    };
                    scroll_to(hwnd, target);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                match (wp.0 & 0xffff) as u32 {
                    1 => crate::native::command(crate::native::SAVE_SETTINGS),
                    2 => {
                        let _ = DestroyWindow(hwnd);
                    }
                    3 => crate::native::command(crate::native::RESET_SETTINGS),
                    _ => {}
                };
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}

pub struct SettingsWindow {
    pub hwnd: HWND,
    scale: Cell<f32>,
    font: Cell<HFONT>,
    wheel_remainder: Cell<i32>,
}
impl SettingsWindow {
    pub fn update_dpi(&self, dpi: u32, bounds: RECT) {
        // SAFETY: this form and its child controls belong to the current UI thread.
        unsafe {
            scroll_to(self.hwnd, 0);
            self.wheel_remainder.set(0);
            let scale = dpi.max(96) as f32 / 96.0;
            let font = CreateFontW(
                -(16.0 * scale) as i32,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            if font.is_invalid() {
                return;
            }
            let mut child = GetWindow(self.hwnd, GW_CHILD).unwrap_or_default();
            while !child.is_invalid() {
                let mut rect = RECT::default();
                let _ = GetWindowRect(child, &mut rect);
                let mut points = [
                    POINT {
                        x: rect.left,
                        y: rect.top,
                    },
                    POINT {
                        x: rect.right,
                        y: rect.bottom,
                    },
                ];
                MapWindowPoints(None, Some(self.hwnd), &mut points);
                let ratio = scale / self.scale.get();
                let _ = SetWindowPos(
                    child,
                    None,
                    (points[0].x as f32 * ratio).round() as i32,
                    (points[0].y as f32 * ratio).round() as i32,
                    ((points[1].x - points[0].x) as f32 * ratio).round() as i32,
                    ((points[1].y - points[0].y) as f32 * ratio).round() as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_NOCOPYBITS,
                );
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(0)),
                );
                child = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
            }
            let _ = DeleteObject(self.font.replace(font).into());
            self.scale.set(scale);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, (670.0 * scale) as isize);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            scroll_range(self.hwnd);
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }
    pub fn new(owner: Option<HWND>, settings: &Settings) -> Result<Box<Self>> {
        // SAFETY: all controls belong to this thread and are destroyed with their parent. Font lives for the form lifetime.
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                hIcon: LoadIconW(
                    Some(instance.into()),
                    PCWSTR(std::ptr::without_provenance(1)),
                )?,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut _),
                lpszClassName: w!("CursorCueSettingsClass"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_thread());
            }
            let scale = owner
                .map(|window| GetDpiForWindow(window) as f32 / 96.0)
                .unwrap_or(1.0)
                .max(1.0);
            let monitor = MonitorFromWindow(owner.unwrap_or_default(), MONITOR_DEFAULTTOPRIMARY);
            let mut monitor_info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                return Err(Error::from_thread());
            }
            let (x, y, width, height) = form_geometry(monitor_info.rcWork, scale);
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.lpszClassName,
                w!("CursorCue Settings"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VSCROLL | WS_CLIPCHILDREN,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let font = CreateFontW(
                -(16.0 * scale) as i32,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let form = Box::new(Self {
                hwnd,
                scale: Cell::new(scale),
                font: Cell::new(font),
                wheel_remainder: Cell::new(0),
            });
            SetPropW(
                hwnd,
                w!("CursorCueSettings"),
                Some(HANDLE((&*form as *const Self).cast_mut().cast())),
            )?;
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).ok();
            form.label("Shared cursor", 24, 18)?;
            for (id, label, y) in [
                (100, "Cursor size (%)", 56),
                (101, "Opacity (%)", 94),
                (102, "Style", 132),
                (103, "Smoothing", 170),
                (105, "Resume duration (ms)", 246),
            ] {
                form.label(label, 24, y + 4)?;
                if id == 102 || id == 103 {
                    form.control(
                        w!("COMBOBOX"),
                        "",
                        id,
                        220,
                        y,
                        210,
                        160,
                        WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                    )?;
                } else {
                    form.control(
                        w!("EDIT"),
                        "",
                        id,
                        220,
                        y,
                        120,
                        28,
                        WS_BORDER | WINDOW_STYLE(ES_NUMBER as u32),
                    )?;
                }
            }
            form.control(
                w!("STATIC"),
                "50-300% (100% = normal)",
                0,
                354,
                60,
                264,
                24,
                WINDOW_STYLE::default(),
            )?;
            form.control(
                w!("STATIC"),
                "20-100%",
                0,
                354,
                98,
                264,
                24,
                WINDOW_STYLE::default(),
            )?;
            form.control(
                w!("BUTTON"),
                "Animate resume",
                104,
                220,
                208,
                210,
                28,
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
            )?;
            form.label("Global shortcuts", 24, 292)?;
            for (index, label) in [
                "Freeze",
                "Hide / reveal",
                "Resume",
                "Drop",
                "Toggle CursorCue",
            ]
            .iter()
            .enumerate()
            {
                let y = 326 + index as i32 * 36;
                form.label(label, 24, y + 4)?;
                let combo = form.control(
                    w!("COMBOBOX"),
                    "",
                    110 + index as u32,
                    220,
                    y,
                    210,
                    210,
                    WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                )?;
                for (_, label) in MODIFIERS {
                    let label = wide(label);
                    SendMessageW(
                        combo,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(label.as_ptr() as isize)),
                    );
                }
                form.control(
                    w!("EDIT"),
                    "",
                    120 + index as u32,
                    452,
                    y,
                    120,
                    28,
                    WS_BORDER | WINDOW_STYLE(ES_UPPERCASE as u32),
                )?;
            }
            for (id, items) in [
                (102, vec!["Arrow", "Dot", "Circle"]),
                (103, vec!["Off", "Light", "Medium", "Strong"]),
            ] {
                let combo = form.item(id)?;
                for label in items {
                    let label = wide(label);
                    SendMessageW(
                        combo,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(label.as_ptr() as isize)),
                    );
                }
            }
            form.control(
                w!("STATIC"),
                "Shortcut key: letter, number, or F1-F24. Clear to disable.",
                0,
                24,
                516,
                600,
                24,
                WINDOW_STYLE::default(),
            )?;
            form.control(
                w!("STATIC"),
                "",
                130,
                24,
                548,
                600,
                64,
                WINDOW_STYLE::default(),
            )?;
            form.control(
                w!("BUTTON"),
                "Reset defaults",
                3,
                24,
                620,
                145,
                32,
                WINDOW_STYLE(BS_PUSHBUTTON as u32),
            )?;
            form.control(
                w!("BUTTON"),
                "Close",
                2,
                342,
                620,
                105,
                32,
                WINDOW_STYLE(BS_PUSHBUTTON as u32),
            )?;
            form.control(
                w!("BUTTON"),
                "Apply",
                1,
                466,
                620,
                105,
                32,
                WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            )?;
            form.populate(settings)?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (670.0 * scale) as isize);
            scroll_range(hwnd);
            Ok(form)
        }
    }
    fn label(&self, label: &str, x: i32, y: i32) -> Result<()> {
        self.control(
            w!("STATIC"),
            label,
            0,
            x,
            y,
            180,
            24,
            WINDOW_STYLE::default(),
        )?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn control(
        &self,
        class: PCWSTR,
        label: &str,
        id: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        style: WINDOW_STYLE,
    ) -> Result<HWND> {
        let label = wide(label);
        // SAFETY: terminated strings live across creation; child HWND and font remain owned by this form.
        unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR(label.as_ptr()),
                WS_CHILD
                    | WS_VISIBLE
                    | if id > 0 && id != 130 {
                        WS_TABSTOP
                    } else {
                        WINDOW_STYLE::default()
                    }
                    | style,
                (x as f32 * self.scale.get()) as i32,
                (y as f32 * self.scale.get()) as i32,
                (width as f32 * self.scale.get()) as i32,
                (height as f32 * self.scale.get()) as i32,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut _)),
                None,
                None,
            )?;
            SendMessageW(
                window,
                WM_SETFONT,
                Some(WPARAM(self.font.get().0 as usize)),
                Some(LPARAM(1)),
            );
            Ok(window)
        }
    }
    fn item(&self, id: i32) -> Result<HWND> {
        // SAFETY: the form owns its child controls; Windows validates both the form handle and child ID.
        unsafe { GetDlgItem(Some(self.hwnd), id) }
    }
    fn text(&self, id: i32) -> Result<String> {
        let mut text = [0u16; 64];
        // SAFETY: the writable UTF-16 buffer lives through GetWindowTextW and is sized by the slice.
        unsafe {
            let count = GetWindowTextW(self.item(id)?, &mut text);
            Ok(String::from_utf16_lossy(&text[..count as usize]))
        }
    }
    pub fn populate(&self, settings: &Settings) -> Result<()> {
        // SAFETY: fields are live child controls. Message payload strings remain alive until synchronous SendMessage returns.
        unsafe {
            for (id, value) in [
                (100, (settings.cursor_scale * 100.0).round().to_string()),
                (101, (settings.opacity * 100.0).round().to_string()),
                (105, settings.animation_duration_ms.to_string()),
            ] {
                let text = wide(&value);
                SetWindowTextW(self.item(id)?, PCWSTR(text.as_ptr()))?;
            }
            SendMessageW(
                self.item(102)?,
                CB_SETCURSEL,
                Some(WPARAM(match settings.cursor_style {
                    CursorStyle::Arrow => 0,
                    CursorStyle::Dot => 1,
                    CursorStyle::Circle => 2,
                })),
                None,
            );
            SendMessageW(
                self.item(103)?,
                CB_SETCURSEL,
                Some(WPARAM(match settings.smoothing {
                    Smoothing::Off => 0,
                    Smoothing::Light => 1,
                    Smoothing::Medium => 2,
                    Smoothing::Strong => 3,
                })),
                None,
            );
            SendMessageW(
                self.item(104)?,
                BM_SETCHECK,
                Some(WPARAM(settings.animation_enabled as usize)),
                None,
            );
            for (index, key) in settings.hotkeys.iter().enumerate() {
                let selected = MODIFIERS
                    .iter()
                    .position(|item| item.0 == key.modifiers)
                    .unwrap_or(0);
                SendMessageW(
                    self.item(110 + index as i32)?,
                    CB_SETCURSEL,
                    Some(WPARAM(selected)),
                    None,
                );
                let text = wide(&key_label(key.key));
                SetWindowTextW(self.item(120 + index as i32)?, PCWSTR(text.as_ptr()))?;
            }
        }
        Ok(())
    }
    pub fn read(&self) -> Result<Settings> {
        let mut settings = Settings {
            cursor_scale: self
                .text(100)?
                .parse::<u32>()
                .map_err(|_| invalid("Size must be 50–300%."))? as f32
                / 100.0,
            opacity: self
                .text(101)?
                .parse::<u32>()
                .map_err(|_| invalid("Opacity must be 20–100%."))? as f32
                / 100.0,
            animation_duration_ms: self
                .text(105)?
                .parse::<u32>()
                .map_err(|_| invalid("Resume duration must be 120–450 milliseconds."))?,
            ..Settings::default()
        };
        // SAFETY: these synchronous messages read state from the form's live native controls.
        unsafe {
            settings.cursor_style = match SendMessageW(self.item(102)?, CB_GETCURSEL, None, None).0
            {
                0 => CursorStyle::Arrow,
                1 => CursorStyle::Dot,
                2 => CursorStyle::Circle,
                _ => return Err(invalid("Choose a cursor style.")),
            };
            settings.smoothing = match SendMessageW(self.item(103)?, CB_GETCURSEL, None, None).0 {
                0 => Smoothing::Off,
                1 => Smoothing::Light,
                2 => Smoothing::Medium,
                3 => Smoothing::Strong,
                _ => return Err(invalid("Choose a smoothing level.")),
            };
            settings.animation_enabled =
                SendMessageW(self.item(104)?, BM_GETCHECK, None, None).0 == 1;
            for (index, hotkey) in settings.hotkeys.iter_mut().enumerate() {
                let text = self.text(120 + index as i32)?.trim().to_ascii_uppercase();
                if text.is_empty() {
                    *hotkey = Hotkey {
                        modifiers: 0,
                        key: 0,
                    };
                    continue;
                }
                let key = if text.len() == 1 && text.as_bytes()[0].is_ascii_alphanumeric() {
                    text.as_bytes()[0] as u32
                } else if let Some(value) = text
                    .strip_prefix('F')
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| (1..=24).contains(value))
                {
                    0x70 + value - 1
                } else if let Some(value) = text
                    .strip_prefix("0X")
                    .and_then(|value| u32::from_str_radix(value, 16).ok())
                {
                    value
                } else {
                    return Err(invalid(
                        "Shortcut keys must be a letter, number, or F1–F24.",
                    ));
                };
                let selected =
                    SendMessageW(self.item(110 + index as i32)?, CB_GETCURSEL, None, None).0;
                let modifiers = MODIFIERS
                    .get(selected as usize)
                    .ok_or_else(|| invalid("Choose shortcut modifiers."))?
                    .0;
                *hotkey = Hotkey { modifiers, key };
            }
        }
        if !(0.5..=3.0).contains(&settings.cursor_scale) {
            return Err(invalid("Cursor size must be 50-300%."));
        }
        if !(0.2..=1.0).contains(&settings.opacity) {
            return Err(invalid("Opacity must be 20-100%."));
        }
        if !(120..=450).contains(&settings.animation_duration_ms) {
            return Err(invalid("Resume duration must be 120-450 milliseconds."));
        }
        settings
            .validate()
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(settings)
    }
    pub fn show(&self) {
        // SAFETY: this native form HWND belongs to this thread for its entire lifetime.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
            if !IsChild(self.hwnd, GetFocus()).as_bool()
                && let Ok(field) = self.item(100)
            {
                let _ = SetFocus(Some(field));
            }
        }
    }
    pub fn set_status(&self, message: &str) {
        if let Ok(control) = self.item(130) {
            let value = wide(message);
            unsafe {
                let _ = SetWindowTextW(control, PCWSTR(value.as_ptr()));
            }
        }
    }
    pub fn reveal_focus(&self) {
        // SAFETY: focus and child geometry are queried only for this thread's live form.
        unsafe {
            let focus = GetFocus();
            if !IsChild(self.hwnd, focus).as_bool() {
                return;
            }
            let mut rect = RECT::default();
            if GetWindowRect(focus, &mut rect).is_err() {
                return;
            }
            let mut top = POINT {
                x: rect.left,
                y: rect.top,
            };
            let mut bottom = POINT {
                x: rect.right,
                y: rect.bottom,
            };
            let _ = ScreenToClient(self.hwnd, &mut top);
            let _ = ScreenToClient(self.hwnd, &mut bottom);
            let mut client = RECT::default();
            if GetClientRect(self.hwnd, &mut client).is_err() {
                return;
            }
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_POS,
                ..Default::default()
            };
            if GetScrollInfo(self.hwnd, SB_VERT, &mut info).is_ok() {
                if top.y < 0 {
                    scroll_to(self.hwnd, info.nPos + top.y - 8);
                } else if bottom.y > client.bottom {
                    scroll_to(self.hwnd, info.nPos + bottom.y - client.bottom + 8);
                }
            }
        }
    }
}
impl Drop for SettingsWindow {
    fn drop(&mut self) {
        // SAFETY: the form owns its HWND and font. Children are destroyed before their shared font is deleted.
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = RemovePropW(self.hwnd, w!("CursorCueSettings"));
                let _ = DestroyWindow(self.hwnd);
            }
            let _ = DeleteObject(self.font.get().into());
        }
    }
}
fn key_label(key: u32) -> String {
    match key {
        0 => String::new(),
        0x70..=0x87 => format!("F{}", key - 0x70 + 1),
        0x30..=0x39 | 0x41..=0x5a => char::from_u32(key).map(String::from).unwrap_or_default(),
        _ => format!("0x{key:X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precision_wheel_input_scrolls_settings() {
        let form = SettingsWindow::new(None, &Settings::default()).unwrap();
        unsafe {
            SetWindowPos(form.hwnd, None, 0, 0, 670, 350, SWP_NOZORDER | SWP_NOMOVE).unwrap();
            for _ in 0..4 {
                SendMessageW(
                    form.hwnd,
                    WM_MOUSEWHEEL,
                    Some(WPARAM((-30i16 as u16 as usize) << 16)),
                    None,
                );
            }
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_POS,
                ..Default::default()
            };
            GetScrollInfo(form.hwnd, SB_VERT, &mut info).unwrap();
            assert!(info.nPos > 0);
        }
    }
    #[test]
    fn settings_restore_reopen_and_rescale_without_losing_values() {
        let form = SettingsWindow::new(None, &Settings::default()).unwrap();
        form.show();
        unsafe {
            assert!(IsWindowVisible(form.hwnd).as_bool());
            assert!(
                GetWindow(form.hwnd, GW_OWNER)
                    .unwrap_or_default()
                    .is_invalid()
            );
            let _ = ShowWindow(form.hwnd, SW_MINIMIZE);
        }
        form.show();
        unsafe {
            assert!(!IsIconic(form.hwnd).as_bool());
        }
        let before = form.read().unwrap();
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 1300,
            bottom: 900,
        };
        unsafe {
            SendMessageW(
                form.hwnd,
                WM_DPICHANGED,
                Some(WPARAM(192 | (192 << 16))),
                Some(LPARAM((&bounds as *const RECT) as isize)),
            );
        }
        let mut field = RECT::default();
        unsafe {
            GetWindowRect(form.item(100).unwrap(), &mut field).unwrap();
        }
        assert_eq!(field.right - field.left, 240);
        assert_eq!(form.read().unwrap(), before);
        unsafe {
            let _ = ShowWindow(form.hwnd, SW_HIDE);
        }
        form.show();
        unsafe {
            assert!(IsWindowVisible(form.hwnd).as_bool());
        }
    }
    #[test]
    fn scaled_form_fits_1080p_work_area() {
        let (x, y, width, height) = form_geometry(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            2.0,
        );
        assert!(x >= 0 && y >= 0 && x + width <= 1920 && y + height <= 1040);
    }
    #[test]
    fn native_scroll_reveals_save_button_in_short_viewport() {
        let form = SettingsWindow::new(None, &Settings::default()).expect("native form");
        unsafe {
            SetWindowPos(form.hwnd, None, 0, 0, 670, 350, SWP_NOZORDER | SWP_NOMOVE)
                .expect("short viewport");
            SendMessageW(
                form.hwnd,
                WM_VSCROLL,
                Some(WPARAM(SB_BOTTOM.0 as usize)),
                None,
            );
            let mut button = RECT::default();
            GetWindowRect(form.item(1).expect("save"), &mut button).expect("button bounds");
            let mut bottom = POINT {
                x: button.right,
                y: button.bottom,
            };
            let _ = ScreenToClient(form.hwnd, &mut bottom);
            let mut client = RECT::default();
            GetClientRect(form.hwnd, &mut client).expect("client");
            assert!(bottom.y <= client.bottom && bottom.y > 0);
        }
    }
    #[test]
    fn native_controls_return_edited_values_and_disabled_shortcuts() {
        let form = SettingsWindow::new(None, &Settings::default()).expect("native form");
        // SAFETY: test owns the real controls; synchronous edit messages exercise the actual form parser.
        unsafe {
            SetWindowTextW(form.item(100).expect("size"), w!("175")).expect("edit size");
            SetWindowTextW(form.item(120).expect("freeze key"), w!("")).expect("clear key");
            SendMessageW(
                form.item(102).expect("style"),
                CB_SETCURSEL,
                Some(WPARAM(2)),
                None,
            );
        }
        let result = form.read().expect("read controls");
        assert_eq!(result.cursor_scale, 1.75);
        assert_eq!(result.cursor_style, CursorStyle::Circle);
        assert_eq!(
            result.hotkeys[0],
            Hotkey {
                modifiers: 0,
                key: 0
            }
        );
        unsafe {
            SetWindowTextW(form.item(100).expect("size"), w!("999")).expect("invalid size");
        }
        assert!(form.read().is_err());
    }
}
