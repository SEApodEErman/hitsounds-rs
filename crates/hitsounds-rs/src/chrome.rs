//! D-57: extend the egui client into the caption while retaining Win32 window styles.

use eframe::egui::{Context, Rect, Vec2};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use std::{cell::RefCell, mem::size_of, num::NonZeroIsize, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
            MonitorFromWindow, ScreenToClient,
        },
    },
    UI::{
        HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi},
        Input::KeyboardAndMouse::{
            TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent, VK_SPACE,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const SUBCLASS_ID: usize = 0x4853_5354;

/// Native file dialogs use the live root window as owner. The shell waits for the
/// worker before closing it. The integer handle is safe to send to the dialog thread.
#[derive(Clone, Copy)]
pub struct Owner(NonZeroIsize);

impl Owner {
    pub fn new(window: &impl HasWindowHandle) -> Result<Self, HandleError> {
        match window.window_handle()?.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(Self(handle.hwnd)),
            _ => Err(HandleError::NotSupported),
        }
    }

    fn hwnd(self) -> HWND {
        self.0.get() as HWND
    }
}

impl HasWindowHandle for Owner {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the owner remains alive while a worker dialog is running.
        Ok(unsafe { WindowHandle::borrow_raw(Win32WindowHandle::new(self.0).into()) })
    }
}

impl HasDisplayHandle for Owner {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

struct State {
    hwnd: HWND,
    ctx: Context,
    drag: RECT,
    max: RECT,
    max_hover: bool,
    alt_space: bool,
    reduce_motion: bool,
    /// Minimum client size in physical pixels (D-63, scaled by D-68). Zero = winit's.
    min_size: POINT,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn install(owner: Owner, ctx: Context) -> bool {
    let hwnd = owner.hwnd();
    // SAFETY: installing on the live window's own thread, with the Win32 callback ABI.
    unsafe {
        let mut client = RECT::default();
        let have_size = GetClientRect(hwnd, &mut client) != 0;
        if SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) == 0 {
            return false;
        }
        STATE.set(Some(State {
            hwnd,
            ctx,
            drag: RECT::default(),
            max: RECT::default(),
            max_hover: false,
            alt_space: false,
            reduce_motion: system_reduces_motion(),
            min_size: POINT::default(),
        }));
        let dark = 1_i32;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const dark).cast(),
            4,
        );
        if IsZoomed(hwnd) != 0 {
            // eframe restores a maximized window hidden, with WS_MAXIMIZE already set, so
            // winit's SW_MAXIMIZE on show changes nothing. Shrinking it to its client
            // size here would leave it zoomed but short of the right and bottom edges.
            fit_maximized(hwnd);
        } else {
            let mut flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
            if !have_size {
                flags |= SWP_NOSIZE;
            }
            SetWindowPos(hwnd, null_mut(), 0, 0, client.right, client.bottom, flags);
        }
    }
    true
}

/// Gives a zoomed window the rectangle Windows maximizes a resizable window to: the
/// monitor's work area plus the resize frame on every side. `WM_NCCALCSIZE` then fits
/// the client to the work area.
///
/// # Safety
/// `hwnd` must be a live window owned by this thread.
unsafe fn fit_maximized(hwnd: HWND) {
    // SAFETY: the caller passes a live window; the output structs are live.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, flags | SWP_NOMOVE | SWP_NOSIZE);
            return;
        }
        let dpi = GetDpiForWindow(hwnd);
        // winit sized the restore rectangle as the saved client plus a standard frame,
        // which this window doesn't have; trim it so Restore gives the saved size.
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        let mut frame = RECT::default();
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & !WS_MAXIMIZE;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if GetWindowPlacement(hwnd, &mut placement) != 0
            && AdjustWindowRectExForDpi(&mut frame, style, 0, ex_style, dpi) != 0
        {
            let normal = &mut placement.rcNormalPosition;
            normal.right -= frame.right - frame.left;
            normal.bottom -= frame.bottom - frame.top;
            // Still hidden: eframe shows the window after its first frame.
            placement.showCmd = SW_HIDE as u32;
            SetWindowPlacement(hwnd, &placement);
        }
        let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        let x = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + pad;
        let y = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + pad;
        let work = info.rcWork;
        SetWindowPos(
            hwnd,
            null_mut(),
            work.left - x,
            work.top - y,
            work.right - work.left + 2 * x,
            work.bottom - work.top + 2 * y,
            flags,
        );
    }
}

pub fn layout(drag: Rect, max: Rect, pixels_per_point: f32) {
    let to_px = |r: Rect| RECT {
        left: (r.min.x * pixels_per_point).round() as i32,
        top: (r.min.y * pixels_per_point).round() as i32,
        right: (r.max.x * pixels_per_point).round() as i32,
        bottom: (r.max.y * pixels_per_point).round() as i32,
    };
    STATE.with_borrow_mut(|s| {
        if let Some(s) = s {
            s.drag = to_px(drag);
            s.max = to_px(max);
        }
    });
}

/// Sets the minimum window size in points. winit adds the standard frame to its own
/// minimum, which this window doesn't have, so resizing to it grows the window; the
/// subclass answers `WM_GETMINMAXINFO` itself instead. False when not installed.
pub fn set_min_size(size: Vec2, pixels_per_point: f32) -> bool {
    STATE.with_borrow_mut(|s| {
        s.as_mut().is_some_and(|s| {
            s.min_size = POINT {
                x: (size.x * pixels_per_point).round() as i32,
                y: (size.y * pixels_per_point).round() as i32,
            };
            true
        })
    })
}

/// Where and how big the window is when not maximized, in physical pixels (outer
/// rectangle, which is also the client here). `None` unless it is maximized now.
///
/// eframe saves the maximized size as the size to restore to, so a maximized
/// session would come back from Restore taller than the work area. The shell saves
/// this instead.
pub fn restore_rect() -> Option<Rect> {
    let hwnd = STATE.with_borrow(|s| s.as_ref().map(|s| s.hwnd))?;
    // SAFETY: the subclass state is cleared on WM_NCDESTROY, so hwnd is live; the
    // output structs are live.
    unsafe {
        if IsZoomed(hwnd) == 0 {
            return None;
        }
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if GetWindowPlacement(hwnd, &mut placement) == 0 {
            return None;
        }
        // The placement is in workspace coordinates: relative to the work area of the
        // monitor the window was restored on.
        let monitor = MonitorFromRect(&placement.rcNormalPosition, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let (dx, dy) = if GetMonitorInfoW(monitor, &mut info) != 0 {
            (
                info.rcWork.left - info.rcMonitor.left,
                info.rcWork.top - info.rcMonitor.top,
            )
        } else {
            (0, 0)
        };
        let r = placement.rcNormalPosition;
        Some(Rect::from_min_max(
            eframe::egui::pos2((r.left + dx) as f32, (r.top + dy) as f32),
            eframe::egui::pos2((r.right + dx) as f32, (r.bottom + dy) as f32),
        ))
    }
}

/// Physical pixels per logical pixel of the window's monitor, as winit reports it.
pub fn scale_factor() -> Option<f32> {
    let hwnd = STATE.with_borrow(|s| s.as_ref().map(|s| s.hwnd))?;
    // SAFETY: as in `restore_rect`.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    (dpi > 0).then(|| dpi as f32 / 96.0)
}

pub fn maximize_hovered() -> bool {
    STATE.with_borrow(|s| s.as_ref().is_some_and(|s| s.max_hover))
}

pub fn take_alt_space() -> bool {
    STATE.with_borrow_mut(|s| s.as_mut().is_some_and(|s| std::mem::take(&mut s.alt_space)))
}

pub fn reduce_motion() -> bool {
    STATE.with_borrow(|s| {
        s.as_ref()
            .map_or_else(system_reduces_motion, |s| s.reduce_motion)
    })
}

fn system_reduces_motion() -> bool {
    let mut enabled = 1_i32;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL into the supplied buffer.
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0) };
    enabled == 0
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    // SAFETY: message payloads come from Win32 and are used only for their documented
    // message types. Call the next subclass with no outstanding borrow of STATE.
    unsafe {
        match msg {
            WM_NCCALCSIZE if wparam != 0 => {
                if IsZoomed(hwnd) != 0 {
                    // A maximized HWND extends beyond the monitor by the resize frame.
                    // Fit the drawable client to the monitor's work area.
                    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    if GetMonitorInfoW(monitor, &mut info) != 0 {
                        (*(lparam as *mut NCCALCSIZE_PARAMS)).rgrc[0] = info.rcWork;
                    }
                }
                return 0;
            }
            WM_GETMINMAXINFO => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                let min = STATE.with_borrow(|s| s.as_ref().map(|s| s.min_size));
                if let Some(min) = min.filter(|m| m.x > 0 && m.y > 0) {
                    // The client covers the whole window (WM_NCCALCSIZE above).
                    (*(lparam as *mut MINMAXINFO)).ptMinTrackSize = min;
                }
                return result;
            }
            WM_NCACTIVATE => return DefSubclassProc(hwnd, msg, wparam, -1),
            WM_NCPAINT => return 0,
            WM_NCHITTEST => return hit_test(hwnd, lparam) as LRESULT,
            WM_NCMOUSEMOVE => {
                if set_max_hover(wparam == HTMAXBUTTON as usize) && wparam == HTMAXBUTTON as usize {
                    TrackMouseEvent(&mut TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    });
                }
            }
            WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
                set_max_hover(false);
            }
            // D-46: Alt+Space follows the transport shortcut. Right-clicking the
            // caption still reaches the native system menu.
            WM_SYSKEYDOWN if wparam == VK_SPACE as usize && lparam & (1 << 29) != 0 => {
                if lparam & (1 << 30) == 0 {
                    STATE.with_borrow_mut(|s| {
                        if let Some(s) = s {
                            s.alt_space = true;
                            s.ctx.request_repaint();
                        }
                    });
                }
                return 0;
            }
            WM_SYSCHAR if wparam == VK_SPACE as usize && lparam & (1 << 29) != 0 => return 0,
            WM_SETTINGCHANGE => STATE.with_borrow_mut(|s| {
                if let Some(s) = s {
                    s.reduce_motion = system_reduces_motion();
                    s.ctx.request_repaint();
                }
            }),
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(window_proc), id);
                STATE.set(None);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

fn set_max_hover(hover: bool) -> bool {
    STATE.with_borrow_mut(|state| {
        if let Some(state) = state
            && state.max_hover != hover
        {
            state.max_hover = hover;
            state.ctx.request_repaint();
            return true;
        }
        false
    })
}

fn hit_test(hwnd: HWND, lparam: LPARAM) -> u32 {
    // Coordinates are signed because a secondary display may be left of the primary.
    let mut point = POINT {
        x: (lparam as i16) as i32,
        y: ((lparam >> 16) as i16) as i32,
    };
    // SAFETY: hwnd is the window being hit-tested and the output structs are live.
    unsafe {
        if IsZoomed(hwnd) == 0 {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect) != 0 {
                let dpi = GetDpiForWindow(hwnd);
                let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                let width = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + pad;
                let height = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + pad;
                let x = if point.x < rect.left + width {
                    -1
                } else if point.x >= rect.right - width {
                    1
                } else {
                    0
                };
                let y = if point.y < rect.top + height {
                    -1
                } else if point.y >= rect.bottom - height {
                    1
                } else {
                    0
                };
                let edge = match (x, y) {
                    (-1, -1) => HTTOPLEFT,
                    (1, -1) => HTTOPRIGHT,
                    (-1, 1) => HTBOTTOMLEFT,
                    (1, 1) => HTBOTTOMRIGHT,
                    (-1, 0) => HTLEFT,
                    (1, 0) => HTRIGHT,
                    (0, -1) => HTTOP,
                    (0, 1) => HTBOTTOM,
                    _ => HTCLIENT,
                };
                if edge != HTCLIENT {
                    return edge;
                }
            }
        }
        ScreenToClient(hwnd, &mut point);
    }
    let contains =
        |r: &RECT| (r.left..r.right).contains(&point.x) && (r.top..r.bottom).contains(&point.y);
    STATE.with_borrow(|s| match s.as_ref() {
        Some(s) if contains(&s.max) => HTMAXBUTTON,
        Some(s) if contains(&s.drag) => HTCAPTION,
        _ => HTCLIENT,
    })
}
