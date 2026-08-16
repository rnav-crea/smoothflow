#![cfg(target_os = "macos")]

//! macOS-only CGEventTap backend for the bare-Fn hotkey.
//!
//! The global-shortcut crate's Carbon backend can't map the Fn key ("Unknown
//! scancode"), so when the configured hotkey is a bare `Fn` we install an
//! event tap that watches kVK_Function (keycode 63), swallows its key events
//! so the OS never sees them, and drives the same `hotkey_start` /
//! `hotkey_finalize` helpers as the shortcut handler.

use crate::{hotkey_finalize, hotkey_start};
use std::sync::Mutex;
use tauri::AppHandle;

// --- FFI declarations (hand-written, mirror CoreGraphics/CoreFoundation) ---

pub type CGEventMask = u64;
pub type CGEventField = u32;
pub const kCGKeyboardEventKeycode: CGEventField = 9;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum CGEventTapLocation {
    Hid,
    Session,
    AnnotatedSession,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum CGEventTapPlacement {
    HeadInsertEventTap = 0,
    TailAppendEventTap,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum CGEventTapOptions {
    Default = 0x00000000,
    ListenOnly = 0x00000001,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CGEventType {
    KeyDown = 10,
    KeyUp = 11,
    TapDisabledByTimeout = 0xFFFFFFFE,
    TapDisabledByUserInput = 0xFFFFFFFF,
}

pub enum CGEvent {}
pub type CGEventRef = *const CGEvent;
pub type CGEventTapProxy = *const std::ffi::c_void;

pub enum CFMachPort {}
pub type CFMachPortRef = *mut CFMachPort;
pub enum CFRunLoop {}
pub type CFRunLoopRef = *mut CFRunLoop;
pub enum CFRunLoopSource {}
pub type CFRunLoopSourceRef = *mut CFRunLoopSource;
pub enum CFString {}
pub type CFStringRef = *const CFString;
pub enum CFAllocator {}
pub type CFAllocatorRef = *mut CFAllocator;
pub type CFIndex = std::os::raw::c_long;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    etype: CGEventType,
    event: CGEventRef,
    user_info: *const std::ffi::c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *const std::ffi::c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFAllocatorDefault: CFAllocatorRef;

    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRelease(cftype: *const std::ffi::c_void);
}

/// Bit for a CGEventType — matches the C `CGEventMaskBit` macro. The huge
/// TapDisabled* values (0xFFFFFFFE / 0xFFFFFFFF) land on bits 62/63 via the
/// hardware shift instruction's count masking, so mask explicitly: a raw
/// `1 << 0xFFFFFFFF` would be a Rust shift overflow.
const fn mask_bit(t: CGEventType) -> CGEventMask {
    1u64 << (t as CGEventMask & 63)
}

const EVENT_MASK: CGEventMask = mask_bit(CGEventType::KeyDown)
    | mask_bit(CGEventType::KeyUp)
    | mask_bit(CGEventType::TapDisabledByTimeout)
    | mask_bit(CGEventType::TapDisabledByUserInput);

/// kVK_Function
const FN_KEYCODE: i64 = 63;

/// The live tap port, so `stop_fn_tap` can tear it down. Stored as `usize`
/// (raw pointers aren't Send, which a `static Mutex` requires); the pointer
/// round-trips losslessly. Mutex-guarded because the callback (run loop
/// thread) and `stop_fn_tap` (shortcut thread) can race.
static TAP: Mutex<Option<usize>> = Mutex::new(None);

pub fn is_running() -> bool {
    TAP.lock().unwrap().is_some()
}

pub fn start_fn_tap(app: AppHandle) -> Result<(), String> {
    if !crate::accessibility::is_trusted() {
        crate::accessibility::request_once();
        return Err(
            "macOS Accessibility permission required for the Fn hotkey — grant it and restart."
                .into(),
        );
    }
    if is_running() {
        return Ok(());
    }
    // Leak the AppHandle for process lifetime; the callback reads it on the
    // main run loop thread and never frees it.
    let user_info = Box::into_raw(Box::new(app)) as *const std::ffi::c_void;
    let tap = unsafe {
        CGEventTapCreate(
            CGEventTapLocation::Hid,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            EVENT_MASK,
            tap_callback,
            user_info,
        )
    };
    if tap.is_null() {
        // Recover the leaked box so we don't leak on the error path.
        drop(unsafe { Box::from_raw(user_info as *mut AppHandle) });
        return Err("Failed to create CGEventTap for the Fn hotkey.".into());
    }
    let run_loop = unsafe { CFRunLoopGetMain() };
    let source = unsafe { CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) };
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    // ponytail: the run-loop source and the AppHandle box leak for process
    // lifetime, matching the tap's own lifetime (the run loop also retains
    // the source, so a later CFRelease of our reference is moot anyway).
    *TAP.lock().unwrap() = Some(tap as usize);
    Ok(())
}

pub fn stop_fn_tap() {
    if let Some(port) = TAP.lock().unwrap().take() {
        let port = port as CFMachPortRef;
        unsafe {
            CGEventTapEnable(port, false);
            CFMachPortInvalidate(port);
            CFRelease(port as *const std::ffi::c_void);
        }
    }
}

unsafe extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    etype: CGEventType,
    event: CGEventRef,
    user_info: *const std::ffi::c_void,
) -> CGEventRef {
    match etype {
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
            if let Some(port) = *TAP.lock().unwrap() {
                unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
            }
            event
        }
        CGEventType::KeyDown | CGEventType::KeyUp => {
            let keycode = unsafe { CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) };
            if keycode == FN_KEYCODE {
                // user_info is a leaked Box<AppHandle> — always valid here.
                let app = (*(user_info as *const AppHandle)).clone();
                if etype == CGEventType::KeyDown {
                    hotkey_start(&app);
                } else {
                    hotkey_finalize(&app);
                }
                // Swallow the event so the OS never sees the Fn press.
                std::ptr::null()
            } else {
                event
            }
        }
    }
}