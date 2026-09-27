//! TCC probes describe this Desktop process only, never the Runner's authority.
use crate::error::{DesktopError, DesktopResult};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ComputerPermissions {
    pub supported: bool,
    pub foreground: bool,
    pub desktop_accessibility: bool,
    pub desktop_screen_recording: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Accessibility,
    ScreenRecording,
    OpenSettings,
}

pub fn probe() -> ComputerPermissions {
    #[cfg(target_os = "macos")]
    unsafe {
        return ComputerPermissions {
            supported: true,
            foreground: false,
            desktop_accessibility: macos::AXIsProcessTrusted(),
            desktop_screen_recording: macos::CGPreflightScreenCaptureAccess(),
        };
    }
    #[cfg(not(target_os = "macos"))]
    ComputerPermissions {
        supported: false,
        foreground: false,
        desktop_accessibility: false,
        desktop_screen_recording: false,
    }
}

pub fn request(action: PermissionAction) -> DesktopResult<ComputerPermissions> {
    #[cfg(target_os = "macos")]
    {
        match action {
            PermissionAction::Accessibility => unsafe {
                macos::request_accessibility();
            },
            PermissionAction::ScreenRecording => unsafe {
                macos::CGRequestScreenCaptureAccess();
            },
            PermissionAction::OpenSettings => {
                let status = std::process::Command::new("/usr/bin/open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map_err(|_| unavailable())?;
                if !status.success() {
                    return Err(unavailable());
                }
            }
        }
        Ok(probe())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = action;
        Err(unavailable())
    }
}

fn unavailable() -> DesktopError {
    DesktopError::new(
        "computer_permissions_unavailable",
        "System permission controls are unavailable",
        "Open system privacy settings and check the Runner permission owner.",
    )
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
        pub fn CGPreflightScreenCaptureAccess() -> bool;
        pub fn CGRequestScreenCaptureAccess() -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }
    pub unsafe fn request_accessibility() {
        // Static CF objects outlive this dictionary; no retain callbacks required.
        let key = kAXTrustedCheckOptionPrompt;
        let value = kCFBooleanTrue;
        let options = CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &value,
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        if !options.is_null() {
            AXIsProcessTrustedWithOptions(options);
            CFRelease(options);
        }
    }
}
