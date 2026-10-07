use std::cell::Cell;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{SetScrollInfo, ShowScrollBar},
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::GetFocus,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, Result, w},
};

const COPY: [(&str, usize); 9] = [
    ("CursorCue Share", 1),
    (
        "Keep your shared cursor steady in calls, reviews and walkthroughs while your mouse keeps working.",
        0,
    ),
    ("1. Choose your app window", 2),
    ("Select the app you want others to see.", 0),
    ("2. Share the CursorCue window", 2),
    (
        "Choose CursorCue Share as the window to share in your meeting app. Keep both windows unminimized.",
        0,
    ),
    ("3. Work in your original app", 2),
    (
        "Freeze, hide, resume or drop the shared cursor using the Tools menu or your shortcuts.",
        0,
    ),
    (
        "Closing this window keeps CursorCue in the tray. Use Quit CursorCue to exit.",
        0,
    ),
];
const ACTIONS: [(u32, &str); 3] = [
    (10, "&Choose a window..."),
    (19, "Cursor size && &shortcuts..."),
    (22, "&How to use CursorCue"),
];
const PAPER: COLORREF = COLORREF(0xfbf6f4);
const STATIC_NO_PREFIX: WINDOW_STYLE = WINDOW_STYLE(0x80);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn fonts(dpi: u32) -> Result<[HFONT; 3]> {
    let mut fonts = [HFONT::default(); 3];
    for (index, (size, weight)) in [(16, 400), (28, 600), (18, 600)].into_iter().enumerate() {
        fonts[index] = unsafe {
            CreateFontW(
                -(size * dpi as i32 / 96),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        if fonts[index].is_invalid() {
            for font in fonts {
                unsafe {
                    let _ = DeleteObject(font.into());
                }
            }
            return Err(windows::core::Error::new(
                E_FAIL,
                "Windows could not create screen-sharing fonts.",
            ));
        }
    }
    Ok(fonts)
}
struct Layout {
    text: [RECT; 9],
    buttons: [RECT; 3],
    logo: RECT,
    height: i32,
}
pub struct Welcome {
    hwnd: HWND,
    text: [HWND; 9],
    buttons: [HWND; 3],
    fonts: Cell<[HFONT; 3]>,
    dpi: Cell<u32>,
    offset: Cell<i32>,
    wheel_remainder: Cell<i32>,
    live: Cell<bool>,
    laying_out: Cell<bool>,
    brush: HBRUSH,
}
impl Welcome {
    pub fn new(hwnd: HWND) -> Result<Box<Self>> {
        let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
        let mut ui = Box::new(Self {
            hwnd,
            text: [HWND::default(); 9],
            buttons: [HWND::default(); 3],
            fonts: Cell::new(fonts(dpi)?),
            dpi: Cell::new(dpi),
            offset: Cell::new(0),
            wheel_remainder: Cell::new(0),
            live: Cell::new(false),
            laying_out: Cell::new(false),
            brush: unsafe { CreateSolidBrush(PAPER) },
        });
        unsafe {
            let instance = GetModuleHandleW(None)?;
            for (index, (text, font)) in COPY.into_iter().enumerate() {
                let text = wide(text);
                let child = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    PCWSTR(text.as_ptr()),
                    WS_CHILD | WS_VISIBLE | STATIC_NO_PREFIX,
                    0,
                    0,
                    1,
                    1,
                    Some(hwnd),
                    Some(HMENU((100 + index) as *mut _)),
                    Some(instance.into()),
                    None,
                )?;
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(ui.fonts.get()[font].0 as usize)),
                    None,
                );
                ui.text[index] = child;
            }
            for (index, (id, label)) in ACTIONS.into_iter().enumerate() {
                let label = wide(label);
                let child = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    PCWSTR(label.as_ptr()),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WINDOW_STYLE(if index == 0 {
                            BS_DEFPUSHBUTTON
                        } else {
                            BS_PUSHBUTTON
                        } as u32),
                    0,
                    0,
                    1,
                    1,
                    Some(hwnd),
                    Some(HMENU(id as usize as *mut _)),
                    Some(instance.into()),
                    None,
                )?;
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(ui.fonts.get()[0].0 as usize)),
                    None,
                );
                ui.buttons[index] = child;
            }
            SetPropW(
                hwnd,
                w!("CursorCueWelcome"),
                Some(HANDLE((&*ui as *const Self).cast_mut().cast())),
            )?;
        }
        ui.resize();
        Ok(ui)
    }
    fn layout(&self, dc: HDC, width: i32) -> Layout {
        let px = |n: i32| (n as f32 * self.dpi.get() as f32 / 96.0).round() as i32;
        let column = px(720).min((width - px(48)).max(1));
        let left = ((width - column) / 2).max(0);
        let mut layout = Layout {
            text: [RECT::default(); 9],
            buttons: [RECT::default(); 3],
            logo: RECT {
                left,
                top: px(32),
                right: left + px(32),
                bottom: px(64),
            },
            height: 0,
        };
        let measure = |index: usize, x: i32, y: i32| {
            let mut rect = RECT {
                left: x,
                top: y,
                right: left + column,
                bottom: y,
            };
            let mut text = wide(COPY[index].0);
            unsafe {
                let previous = SelectObject(dc, self.fonts.get()[COPY[index].1].into());
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
                );
                SelectObject(dc, previous);
            }
            rect
        };
        layout.text[0] = measure(0, left + px(48), px(29));
        let mut y = layout.text[0].bottom.max(layout.logo.bottom) + px(12);
        layout.text[1] = measure(1, left, y);
        y = layout.text[1].bottom + px(20);
        if column >= px(664) {
            let mut x = left;
            for (index, width) in [210, 250, 160].into_iter().enumerate() {
                layout.buttons[index] = RECT {
                    left: x,
                    top: y,
                    right: x + px(width),
                    bottom: y + px(42),
                };
                x += px(width + 12);
            }
            y += px(42);
        } else {
            for rect in &mut layout.buttons {
                *rect = RECT {
                    left,
                    top: y,
                    right: left + column,
                    bottom: y + px(42),
                };
                y += px(50);
            }
            y -= px(8);
        }
        y += px(28);
        for index in (2..8).step_by(2) {
            layout.text[index] = measure(index, left, y);
            layout.text[index + 1] = measure(index + 1, left, layout.text[index].bottom + px(6));
            y = layout.text[index + 1].bottom + px(20);
        }
        layout.text[8] = measure(8, left, y + px(4));
        layout.height = layout.text[8].bottom + px(28);
        layout
    }
    pub fn resize(&self) {
        if self.live.get() || self.laying_out.replace(true) {
            return;
        }
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            if !dc.is_invalid() {
                for _ in 0..2 {
                    let mut client = RECT::default();
                    if GetClientRect(self.hwnd, &mut client).is_err() {
                        break;
                    }
                    let layout = self.layout(dc, client.right);
                    let offset = self
                        .offset
                        .get()
                        .clamp(0, (layout.height - client.bottom).max(0));
                    self.offset.set(offset);
                    SetScrollInfo(
                        self.hwnd,
                        SB_VERT,
                        &SCROLLINFO {
                            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                            nMin: 0,
                            nMax: layout.height - 1,
                            nPage: client.bottom.max(1) as u32,
                            nPos: offset,
                            ..Default::default()
                        },
                        true,
                    );
                    for (window, rect) in self
                        .text
                        .iter()
                        .zip(layout.text)
                        .chain(self.buttons.iter().zip(layout.buttons))
                    {
                        let _ = MoveWindow(
                            *window,
                            rect.left,
                            rect.top - offset,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            true,
                        );
                    }
                    let mut after = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut after);
                    if after.right == client.right {
                        break;
                    }
                }
                ReleaseDC(Some(self.hwnd), dc);
            }
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
        self.laying_out.set(false);
    }
    pub fn update_dpi(&self, dpi: u32) -> Result<()> {
        let dpi = dpi.max(96);
        if self.dpi.get() != dpi {
            let old = self.fonts.replace(fonts(dpi)?);
            self.dpi.set(dpi);
            self.offset.set(0);
            self.wheel_remainder.set(0);
            unsafe {
                for (index, child) in self.text.iter().enumerate() {
                    SendMessageW(
                        *child,
                        WM_SETFONT,
                        Some(WPARAM(self.fonts.get()[COPY[index].1].0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
                for child in self.buttons {
                    SendMessageW(
                        child,
                        WM_SETFONT,
                        Some(WPARAM(self.fonts.get()[0].0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
                for font in old {
                    let _ = DeleteObject(font.into());
                }
            }
        }
        self.resize();
        Ok(())
    }
    pub fn set_live(&self, live: bool) {
        self.live.set(live);
        self.offset.set(0);
        self.wheel_remainder.set(0);
        unsafe {
            for child in self.text.into_iter().chain(self.buttons) {
                let _ = ShowWindow(child, if live { SW_HIDE } else { SW_SHOWNA });
            }
            if live {
                let _ = ShowScrollBar(self.hwnd, SB_VERT, false);
            }
        }
        if !live {
            self.resize();
        }
    }
    pub fn scroll(&self, action: i32, wheel: i32) {
        if self.live.get() {
            return;
        }
        unsafe {
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            if GetScrollInfo(self.hwnd, SB_VERT, &mut info).is_err() {
                return;
            }
            let line = (32 * self.dpi.get() / 96) as i32;
            let target = if wheel != 0 {
                let delta = self.wheel_remainder.get() + wheel * line * 3;
                self.wheel_remainder.set(delta % 120);
                info.nPos - delta / 120
            } else {
                match action {
                    n if n == SB_LINEUP.0 => info.nPos - line,
                    n if n == SB_LINEDOWN.0 => info.nPos + line,
                    n if n == SB_PAGEUP.0 => info.nPos - info.nPage as i32,
                    n if n == SB_PAGEDOWN.0 => info.nPos + info.nPage as i32,
                    n if n == SB_THUMBTRACK.0 => info.nTrackPos,
                    n if n == SB_TOP.0 => 0,
                    n if n == SB_BOTTOM.0 => info.nMax,
                    _ => info.nPos,
                }
            };
            self.offset.set(target.max(0));
        }
        self.resize();
    }
    pub fn reveal_focus(&self) {
        unsafe {
            let focus = GetFocus();
            if !self.buttons.contains(&focus) {
                return;
            }
            let mut rect = RECT::default();
            if GetWindowRect(focus, &mut rect).is_err() {
                return;
            }
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
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            if points[0].y < 0 {
                self.offset
                    .set((self.offset.get() + points[0].y - 8).max(0));
            } else if points[1].y > client.bottom {
                self.offset
                    .set(self.offset.get() + points[1].y - client.bottom + 8);
            } else {
                return;
            }
        }
        self.resize();
    }
    pub fn paint(&self, dc: HDC) {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            FillRect(dc, &client, self.brush);
            if !self.live.get() {
                let rect = self.layout(dc, client.right).logo;
                if let Ok(instance) = GetModuleHandleW(None)
                    && let Ok(icon) = LoadImageW(
                        Some(instance.into()),
                        PCWSTR(std::ptr::without_provenance(1)),
                        IMAGE_ICON,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        LR_SHARED,
                    )
                {
                    let _ = DrawIconEx(
                        dc,
                        rect.left,
                        rect.top - self.offset.get(),
                        HICON(icon.0),
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        0,
                        None,
                        DI_NORMAL,
                    );
                }
            }
        }
    }
    pub fn static_color(&self, dc: HDC, child: HWND) -> HBRUSH {
        unsafe {
            SetTextColor(
                dc,
                if [100, 102, 104, 106].contains(&GetDlgCtrlID(child)) {
                    COLORREF(0x16110e)
                } else {
                    COLORREF(0x70625c)
                },
            );
            SetBkColor(dc, PAPER);
        }
        self.brush
    }
}
impl Drop for Welcome {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = RemovePropW(self.hwnd, w!("CursorCueWelcome"));
            }
            for child in self.text.into_iter().chain(self.buttons) {
                if IsWindow(Some(child)).as_bool() {
                    let _ = DestroyWindow(child);
                }
            }
            for font in self.fonts.get() {
                let _ = DeleteObject(font.into());
            }
            let _ = DeleteObject(self.brush.into());
        }
    }
}
pub fn with_window<T>(hwnd: HWND, action: impl FnOnce(&Welcome) -> T) -> Option<T> {
    // This private property points to a stable Box owned by the UI thread until HWND destruction.
    unsafe {
        let ui = GetPropW(hwnd, w!("CursorCueWelcome"));
        if ui.is_invalid() {
            None
        } else {
            Some(action(&*(ui.0 as *const Welcome)))
        }
    }
}
