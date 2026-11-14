//! Windows Ink capture before winit consumes pointer history. The Win32 boundary is isolated
//! here; all pressure/contact mapping and queue bounds are safe code in `windows_ink`.
//!
//! Install `message_hook(queue.clone())` with winit's `with_msg_hook`. The hook always returns
//! false: ordinary UI navigation still reaches winit, while the canvas uses the paired samples.
//! Every window seen by the event loop is configured independently, including detached views.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};
use windows_sys::Win32::UI::Controls::POINTER_DEVICE_PROPERTY;
use windows_sys::Win32::UI::Input::Pointer::*;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::Error;
use crate::windows_ink::{PenPhase, PenQueue, RawPenEvent};

const MAX_WINDOWS: usize = 64;
const MAX_HISTORY: usize = 512;
const SUBCLASS_ID: usize = 0x5052_4953;
const WM_TABLET_QUERYSYSTEMGESTURESTATUS: u32 = 0x02cc;
const TABLET_DISABLE_PRESSANDHOLD: usize = 1;
const PEN_PROPERTY: windows_sys::core::PCWSTR = windows_sys::core::w!("MicrosoftTabletPenServiceProperty");

struct WindowState {
    queue: PenQueue,
    previous_property: usize,
    owner: Rc<()>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<usize, WindowState>> = RefCell::new(HashMap::new());
}

/// Lifetime guard captured by the callback. It is !Send, so window cleanup stays on the
/// creating event-loop thread. Native subclass callbacks own no borrowed Rust pointer.
struct Hook {
    queue: PenQueue,
    owner: Rc<()>,
}

impl Drop for Hook {
    fn drop(&mut self) {
        let windows = WINDOWS.with(|all| {
            let Ok(mut all) = all.try_borrow_mut() else { return Vec::new() };
            let keys: Vec<_> = all.iter().filter(|(_, state)| Rc::ptr_eq(&state.owner, &self.owner)).map(|(hwnd, _)| *hwnd).collect();
            keys.into_iter().filter_map(|key| all.remove(&key).map(|state| (key, state))).collect::<Vec<_>>()
        });
        for (key, state) in windows {
            let hwnd = key as HWND;
            // SAFETY: HWNDs were installed on this thread; RemoveWindowSubclass validates
            // the handle and exact callback/id. No user-data pointers are retained.
            unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
            restore_property(hwnd, state.previous_property);
            state.queue.cancel_window(key);
        }
    }
}

/// Safe callback factory for winit `EventLoopBuilderExtWindows::with_msg_hook`.
///
/// Even an arbitrary pointer passed by another caller cannot cause a Rust dereference:
/// ReadProcessMemory validates and copies a MSG through the OS. This keeps `unsafe` entirely
/// inside this crate without exposing a safe API whose soundness depends on its caller.
pub fn message_hook(queue: PenQueue) -> impl FnMut(*const c_void) -> bool + 'static {
    let hook = Hook { queue, owner: Rc::new(()) };
    move |message| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook.handle(message)));
        if result.is_err() {
            hook.queue.report_error(Error::Platform("Windows Ink event callback panicked".into()));
        }
        false
    }
}

impl Hook {
    fn handle(&self, message: *const c_void) {
        let Some(msg) = read_message(message) else { return };
        if msg.hwnd.is_null() {
            return;
        }
        if let Err(error) = self.install_window(msg.hwnd) {
            self.queue.report_error(error);
            return;
        }
        match msg.message {
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => self.capture(msg.hwnd, (msg.wParam & 0xffff) as u32),
            _ => {}
        }
    }

    fn install_window(&self, hwnd: HWND) -> Result<(), Error> {
        let key = hwnd as usize;
        let installed = WINDOWS.with(|all| all.try_borrow().ok().is_some_and(|all| all.contains_key(&key)));
        if installed {
            return Ok(());
        }
        let mut process = 0;
        // SAFETY: process points to initialized stack storage; Win32 validates HWND. Thread
        // affinity prevents cross-thread subclassing or touching another process's windows.
        let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
        // SAFETY: these no-argument queries have no preconditions.
        if thread != unsafe { GetCurrentThreadId() } || process != unsafe { GetCurrentProcessId() } {
            return Err(Error::Platform("Windows Ink ignored a window outside the event-loop thread".into()));
        }
        let capacity = WINDOWS.with(|all| all.try_borrow().ok().is_some_and(|all| all.len() < MAX_WINDOWS));
        if !capacity {
            return Err(Error::Platform("Windows Ink window limit reached".into()));
        }
        // SAFETY: PEN_PROPERTY is a static NUL-terminated string; HWND ownership was checked.
        let previous_property = unsafe { GetPropW(hwnd, PEN_PROPERTY) } as usize;
        // SAFETY: a property is an opaque integer/handle, never dereferenced. Preserve existing
        // flags and suppress only press-and-hold, not real right clicks or barrel buttons.
        if unsafe { SetPropW(hwnd, PEN_PROPERTY, (previous_property | TABLET_DISABLE_PRESSANDHOLD) as _) } == 0 {
            return Err(last_error("SetPropW(tablet press-and-hold)"));
        }
        // SAFETY: callback has the exact Win32 ABI, static lifetime, no raw Rust user data,
        // and runs only on the owning thread. NCDESTROY and Hook::drop remove the subclass.
        if unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) } == 0 {
            let error = last_error("SetWindowSubclass(tablet)");
            restore_property(hwnd, previous_property);
            return Err(error);
        }
        let stored = WINDOWS.with(|all| {
            let Ok(mut all) = all.try_borrow_mut() else { return false };
            all.insert(key, WindowState { queue: self.queue.clone(), previous_property, owner: self.owner.clone() });
            true
        });
        if !stored {
            // SAFETY: remove exactly the subclass just installed on this thread.
            unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
            restore_property(hwnd, previous_property);
            return Err(Error::Platform("Windows Ink window registration was reentered".into()));
        }
        Ok(())
    }

    fn capture(&self, hwnd: HWND, pointer_id: u32) {
        let mut pointer_type = 0;
        // SAFETY: output points to initialized stack storage; OS validates the pointer ID.
        if unsafe { GetPointerType(pointer_id, &mut pointer_type) } == 0 {
            self.queue.report_error(last_error("GetPointerType"));
            self.queue.cancel_pointer(hwnd as usize, pointer_id);
            return;
        }
        if pointer_type != PT_PEN {
            return;
        }
        let mut latest = POINTER_PEN_INFO::default();
        // SAFETY: output is valid, initialized storage for exactly one native structure.
        if unsafe { GetPointerPenInfo(pointer_id, &mut latest) } == 0 {
            self.queue.report_error(last_error("GetPointerPenInfo"));
            self.queue.cancel_pointer(hwnd as usize, pointer_id);
            return;
        }
        if latest.pointerInfo.pointerFlags & POINTER_FLAG_DOWN != 0 && latest.penMask & PEN_MASK_PRESSURE == 0 {
            let supported = pressure_support(latest.pointerInfo.sourceDevice);
            if supported.is_none() {
                self.queue.report_error(Error::Unsupported("Windows Ink device pressure capability is unknown; waiting for a valid pressure packet".into()));
            }
            self.queue.set_pressure_support(latest.pointerInfo.sourceDevice as usize, supported);
        }
        let requested = (latest.pointerInfo.historyCount as usize).clamp(1, MAX_HISTORY);
        let mut history = vec![POINTER_PEN_INFO::default(); requested];
        let mut count = requested as u32;
        // SAFETY: count initially equals the allocated array length. Windows fills at most
        // that many records; it may return a larger total count, which is checked below.
        if unsafe { GetPointerPenInfoHistory(pointer_id, &mut count, history.as_mut_ptr()) } == 0 {
            self.queue.report_error(last_error("GetPointerPenInfoHistory; unfinished stroke canceled"));
            self.queue.cancel_pointer(hwnd as usize, pointer_id);
            return;
        }
        if count as usize > requested {
            self.queue.report_error(Error::Platform("Windows Ink history exceeds bounded buffer; unfinished stroke canceled".into()));
            self.queue.cancel_pointer(hwnd as usize, pointer_id);
            return;
        }
        // Win32 returns newest first; reverse the complete coordinate/pressure records,
        // never query current pressure again while processing historical coordinates.
        for info in history.iter().take(count as usize).rev() {
            let p = &info.pointerInfo;
            let mut position = p.ptPixelLocation;
            // SAFETY: position points to initialized POINT storage and HWND was validated.
            if unsafe { ScreenToClient(hwnd, &mut position) } == 0 {
                self.queue.report_error(last_error("ScreenToClient(pen history)"));
                self.queue.cancel_pointer(hwnd as usize, pointer_id);
                return;
            }
            let flags = p.pointerFlags;
            let phase = if flags & (POINTER_FLAG_CANCELED | POINTER_FLAG_CAPTURECHANGED) != 0 {
                PenPhase::Cancel
            } else if flags & POINTER_FLAG_DOWN != 0 {
                PenPhase::Down
            } else if flags & POINTER_FLAG_UP != 0 {
                PenPhase::Up
            } else if flags & POINTER_FLAG_INCONTACT != 0 {
                PenPhase::Move
            } else {
                PenPhase::Hover
            };
            self.queue.push(RawPenEvent {
                window: hwnd as usize,
                pointer_id: p.pointerId,
                device_id: p.sourceDevice as usize,
                phase,
                x: position.x as f32,
                y: position.y as f32,
                time_ms: p.dwTime,
                pressure: (info.penMask & PEN_MASK_PRESSURE != 0).then_some(info.pressure as f32 / 1024.0),
                tilt_x: (info.penMask & PEN_MASK_TILT_X != 0).then_some(info.tiltX as f32),
                tilt_y: (info.penMask & PEN_MASK_TILT_Y != 0).then_some(info.tiltY as f32),
                rotation: (info.penMask & PEN_MASK_ROTATION != 0).then_some(info.rotation as f32),
                eraser: info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
                barrel_button: info.penFlags & PEN_FLAG_BARREL != 0,
            });
        }
    }
}

fn pressure_support(device: windows_sys::Win32::Foundation::HANDLE) -> Option<bool> {
    if device.is_null() {
        return None;
    }
    let mut count = 0;
    // SAFETY: the OS validates the opaque device handle. A null output with count zero
    // queries required length, as specified by GetPointerDeviceProperties.
    if unsafe { GetPointerDeviceProperties(device, &mut count, std::ptr::null_mut()) } == 0 || count == 0 || count > 256 {
        return None;
    }
    let mut properties = vec![POINTER_DEVICE_PROPERTY::default(); count as usize];
    // SAFETY: allocated output count exactly matches the capacity supplied to Win32. A
    // device-change race returns failure or a larger required count, both rejected below.
    if unsafe { GetPointerDeviceProperties(device, &mut count, properties.as_mut_ptr()) } == 0 || count as usize > properties.len() {
        return None;
    }
    // HID Digitizer usage page 0x0d, Tip Pressure usage 0x30 (Microsoft digitizer descriptor
    // guidance). Absence is only interpreted after a successful complete descriptor query.
    Some(properties.iter().take(count as usize).any(|p| p.usagePageId == 0x0d && p.usageId == 0x30))
}

fn read_message(address: *const c_void) -> Option<MSG> {
    if address.is_null() {
        return None;
    }
    let mut msg = MSG::default();
    let mut copied = 0;
    // SAFETY: the current-process pseudo-handle needs no ownership/close. Destination is
    // exactly sizeof(MSG) initialized stack storage. ReadProcessMemory checks source memory
    // in the kernel, so arbitrary addresses cannot trigger an unchecked Rust dereference.
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), address, (&mut msg as *mut MSG).cast(), size_of::<MSG>(), &mut copied) };
    (ok != 0 && copied == size_of::<MSG>()).then_some(msg)
}

fn last_error(operation: &str) -> Error {
    // SAFETY: GetLastError is a no-argument thread-local query.
    Error::Platform(format!("{operation}: Win32 error {}", unsafe { GetLastError() }))
}

fn restore_property(hwnd: HWND, previous: usize) {
    // SAFETY: static NUL-terminated key; Win32 validates window handles. The prior property
    // is an opaque integer, not a Rust allocation; restore it if another component owned it.
    unsafe {
        if previous == 0 {
            RemovePropW(hwnd, PEN_PROPERTY);
        } else {
            SetPropW(hwnd, PEN_PROPERTY, previous as _);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if message == WM_TABLET_QUERYSYSTEMGESTURESTATUS {
            // SAFETY: Win32 invokes this callback for its live HWND. Preserve the result from
            // the rest of the subclass chain and disable just the long-press gesture.
            return Some(unsafe { DefSubclassProc(hwnd, message, wparam, lparam) } | TABLET_DISABLE_PRESSANDHOLD as LRESULT);
        }
        let queue = WINDOWS.with(|all| all.try_borrow().ok().and_then(|all| all.get(&(hwnd as usize)).map(|state| state.queue.clone())));
        if let Some(queue) = queue {
            match message {
                WM_KILLFOCUS | WM_CANCELMODE => queue.cancel_window(hwnd as usize),
                WM_POINTERCAPTURECHANGED => queue.cancel_pointer(hwnd as usize, (wparam & 0xffff) as u32),
                WM_NCDESTROY => {
                    queue.cancel_window(hwnd as usize);
                    let state = WINDOWS.with(|all| all.try_borrow_mut().ok().and_then(|mut all| all.remove(&(hwnd as usize))));
                    if let Some(state) = state {
                        restore_property(hwnd, state.previous_property);
                    }
                    // SAFETY: remove this exact callback/id before HWND destruction completes.
                    unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
                }
                _ => {}
            }
        }
        None
    }));
    if let Ok(Some(value)) = result {
        return value;
    }
    // SAFETY: this callback passes the native arguments unchanged to the next procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

// ---------------------------------------------------------------------------------------------
// Synthetic pen injection (test tooling): drives the real Windows Ink path without a tablet.

/// A system-wide synthetic pen device (`CreateSyntheticPointerDevice(PT_PEN)`). Injected
/// frames move the real cursor and reach whichever window is under it, exactly like a physical
/// pen: Prism's native capture, or another application used as a comparison oracle.
///
/// Dropping the device lifts a pen left in contact and destroys it.
pub struct SyntheticPen {
    device: windows_sys::Win32::UI::Controls::HSYNTHETICPOINTERDEVICE,
    last: Option<crate::synthetic_pen::PenFrame>,
}

impl SyntheticPen {
    /// Create the device. Also makes this process per-monitor DPI aware ([`make_dpi_aware`]),
    /// so injected frames and [`find_window`] geometry are both physical pixels.
    pub fn new() -> Result<Self, Error> {
        make_dpi_aware();
        // SAFETY: plain value arguments; the returned handle is owned by this struct and
        // destroyed exactly once in Drop.
        let device =
            unsafe { windows_sys::Win32::UI::Controls::CreateSyntheticPointerDevice(PT_PEN, 1, windows_sys::Win32::UI::Controls::POINTER_FEEDBACK_DEFAULT) };
        if device.is_null() {
            return Err(last_error("CreateSyntheticPointerDevice(PT_PEN) (needs Windows 10 1809 or later)"));
        }
        Ok(Self { device, last: None })
    }

    /// Inject one frame now (the caller paces frames with `delay_ms`).
    pub fn inject(&mut self, frame: &crate::synthetic_pen::PenFrame) -> Result<(), Error> {
        let (x, y) = frame.native_position();
        let (tilt_x, tilt_y) = frame.native_tilt();
        let mut info = windows_sys::Win32::UI::Controls::POINTER_TYPE_INFO { r#type: PT_PEN, ..Default::default() };
        let pen = POINTER_PEN_INFO {
            pointerInfo: POINTER_INFO {
                pointerType: PT_PEN,
                pointerFlags: frame.pointer_flags(),
                ptPixelLocation: windows_sys::Win32::Foundation::POINT { x, y },
                ..Default::default()
            },
            penFlags: frame.pen_flags(),
            penMask: PEN_MASK_PRESSURE | PEN_MASK_ROTATION | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
            pressure: frame.native_pressure(),
            rotation: frame.native_rotation(),
            tiltX: tilt_x,
            tiltY: tilt_y,
        };
        info.Anonymous.penInfo = pen;
        // SAFETY: `info` is one fully initialized POINTER_TYPE_INFO whose tag (PT_PEN) matches
        // the written union member; count is 1 and the device handle is live.
        if unsafe { InjectSyntheticPointerInput(self.device, &info, 1) } == 0 {
            return Err(last_error("InjectSyntheticPointerInput"));
        }
        self.last = Some(*frame);
        Ok(())
    }

    /// Inject a whole stroke, sleeping each frame's `delay_ms` first.
    pub fn play(&mut self, stroke: &crate::synthetic_pen::Stroke) -> Result<(), Error> {
        for frame in stroke.frames() {
            if frame.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(u64::from(frame.delay_ms)));
            }
            self.inject(frame)?;
        }
        Ok(())
    }
}

impl Drop for SyntheticPen {
    fn drop(&mut self) {
        use crate::synthetic_pen::{PenFrame, Phase};
        if let Some(last) = self.last.filter(|f| matches!(f.phase, Phase::Down | Phase::Move)) {
            let _ = self.inject(&PenFrame::new(Phase::Up, last.x, last.y, 0.0, 0));
            let _ = self.inject(&PenFrame::new(Phase::Leave, last.x, last.y, 0.0, 0));
        }
        // SAFETY: the handle came from CreateSyntheticPointerDevice and is destroyed once.
        unsafe { windows_sys::Win32::UI::Controls::DestroySyntheticPointerDevice(self.device) };
    }
}

/// Make this process per-monitor DPI aware so window geometry and injected positions share
/// physical pixels. Call before [`find_window`]; harmless if awareness is already set.
pub fn make_dpi_aware() {
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    // SAFETY: takes a predefined pseudo-handle constant. Failure only means awareness was
    // already set (for example by a manifest), which is not an error here.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// A top-level window found by title, with its client area on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowTarget {
    pub handle: usize,
    pub title: String,
    /// Client-area origin in screen pixels.
    pub client_x: i32,
    pub client_y: i32,
    pub client_width: i32,
    pub client_height: i32,
}

thread_local! {
    static ENUMERATED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn collect_window(hwnd: HWND, _: LPARAM) -> windows_sys::core::BOOL {
    ENUMERATED.with(|all| {
        if let Ok(mut all) = all.try_borrow_mut()
            && all.len() < 4096
        {
            all.push(hwnd as usize);
        }
    });
    1
}

/// The visible, non-minimized top-level window titled `needle` (case-insensitive), else the first
/// one whose title contains it. Windows with an empty client area are skipped: a minimized window
/// reports 0×0 at (−32000, −32000), and injecting there lands off screen.
pub fn find_window(needle: &str) -> Result<Option<WindowTarget>, Error> {
    let needle = needle.to_lowercase();
    let found = matching_windows(&needle)?;
    Ok(crate::synthetic_pen::pick_window(found.iter().map(|t| (t.title.as_str(), t.client_width, t.client_height)), &needle)
        .and_then(|i| found.into_iter().nth(i)))
}

fn matching_windows(needle: &str) -> Result<Vec<WindowTarget>, Error> {
    let mut found = Vec::new();
    ENUMERATED.with(|all| all.borrow_mut().clear());
    // SAFETY: the callback has the exact WNDENUMPROC ABI and keeps no pointer; it records
    // handles in a thread-local bounded list on this (the calling) thread.
    if unsafe { EnumWindows(Some(collect_window), 0) } == 0 {
        return Err(last_error("EnumWindows"));
    }
    let handles = ENUMERATED.with(|all| std::mem::take(&mut *all.borrow_mut()));
    for key in handles {
        let hwnd = key as HWND;
        // SAFETY: Win32 validates handles; a window closed meanwhile simply fails the query.
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            continue;
        }
        let mut text = [0u16; 512];
        // SAFETY: the buffer length passed equals the array length.
        let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
        let Some(title) = text.get(..len.max(0) as usize).map(String::from_utf16_lossy) else { continue };
        if !title.to_lowercase().contains(needle) {
            continue;
        }
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        // SAFETY: both outputs are initialized stack storage of the documented types.
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 || unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin) } == 0 {
            continue;
        }
        found.push(WindowTarget {
            handle: key,
            title,
            client_x: origin.x,
            client_y: origin.y,
            client_width: rect.right.saturating_sub(rect.left),
            client_height: rect.bottom.saturating_sub(rect.top),
        });
    }
    Ok(found)
}

/// Ask Windows to bring `target` to the foreground (may be refused by focus-stealing rules;
/// then click the window once before injecting).
pub fn bring_to_front(target: &WindowTarget) -> bool {
    // SAFETY: Win32 validates the handle; a stale handle just returns failure.
    unsafe { SetForegroundWindow(target.handle as HWND) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_window_misses_cleanly() {
        assert_eq!(find_window("no window is titled like this 7f3a9c").ok().flatten(), None);
    }

    #[test]
    fn safe_hook_rejects_invalid_addresses_without_dereferencing() {
        let queue = PenQueue::default();
        let mut hook = message_hook(queue.clone());
        assert!(!hook(std::ptr::null()));
        assert!(!hook(std::ptr::without_provenance(1)));
        let msg = MSG::default();
        assert!(!hook((&msg as *const MSG).cast()));
        assert!(queue.pending_windows().is_empty());
    }

    #[test]
    fn native_window_gesture_policy_focus_cancel_and_cleanup() {
        // SAFETY: predefined STATIC class requires no custom window procedure or registration.
        // All handles stay on this test thread; the hidden window never takes user focus.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                windows_sys::core::w!("STATIC"),
                windows_sys::core::w!("Prism tablet test"),
                0,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        // SAFETY: set an unrelated existing tablet property to verify it survives the hook.
        unsafe { SetPropW(hwnd, PEN_PROPERTY, 0x100usize as _) };
        let queue = PenQueue::default();
        let mut hook = message_hook(queue.clone());
        let msg = MSG { hwnd, message: WM_NULL, ..Default::default() };
        assert!(!hook((&msg as *const MSG).cast()));
        assert!(queue.take_errors().is_empty());
        // SAFETY: SendMessage targets the live hidden test window synchronously on its thread.
        assert_ne!(unsafe { SendMessageW(hwnd, WM_TABLET_QUERYSYSTEMGESTURESTATUS, 0, 0) } & 1, 0);
        queue.push(RawPenEvent {
            window: hwnd as usize,
            pointer_id: 1,
            device_id: 1,
            phase: PenPhase::Down,
            x: 1.0,
            y: 1.0,
            time_ms: 1,
            pressure: Some(0.1),
            tilt_x: None,
            tilt_y: None,
            rotation: None,
            eraser: false,
            barrel_button: false,
        });
        // SAFETY: same synchronous, live test-window call.
        unsafe { SendMessageW(hwnd, WM_KILLFOCUS, 0, 0) };
        assert_eq!(queue.drain(hwnd as usize).last().map(|e| e.phase), Some(PenPhase::Cancel));
        drop(hook);
        // SAFETY: property query and destruction use the single owned test HWND exactly once.
        assert_eq!(unsafe { GetPropW(hwnd, PEN_PROPERTY) } as usize, 0x100);
        // SAFETY: remove this test's property before destroying its window.
        unsafe { RemovePropW(hwnd, PEN_PROPERTY) };
        unsafe { DestroyWindow(hwnd) };
        assert!(WINDOWS.with(|all| !all.borrow().contains_key(&(hwnd as usize))));
    }

    #[test]
    fn destroying_window_before_hook_cancels_and_unregisters() {
        // SAFETY: predefined STATIC window with no visibility/activation flag; owned here.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                windows_sys::core::w!("STATIC"),
                windows_sys::core::w!("Prism tablet lifetime test"),
                0,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        let queue = PenQueue::default();
        let mut hook = message_hook(queue.clone());
        let msg = MSG { hwnd, message: WM_NULL, ..Default::default() };
        hook((&msg as *const MSG).cast());
        queue.push(RawPenEvent {
            window: hwnd as usize,
            pointer_id: 9,
            device_id: 2,
            phase: PenPhase::Down,
            x: 1.0,
            y: 1.0,
            time_ms: 1,
            pressure: Some(0.1),
            tilt_x: None,
            tilt_y: None,
            rotation: None,
            eraser: false,
            barrel_button: false,
        });
        // SAFETY: destroy the one owned HWND; the native NCDESTROY callback removes itself.
        unsafe { DestroyWindow(hwnd) };
        assert_eq!(queue.drain(hwnd as usize).last().map(|e| e.phase), Some(PenPhase::Cancel));
        assert!(WINDOWS.with(|all| !all.borrow().contains_key(&(hwnd as usize))));
        drop(hook);
        assert_eq!(pressure_support(std::ptr::null_mut()), None);
    }
}
