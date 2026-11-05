//! Exact native-display identity -> one physical DDC monitor. No index/name guesses.
use std::{ffi::c_void, mem::size_of};
use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect},
    UI::HiDpi::{DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
};

#[repr(C)]
struct PhysicalMonitor {
    handle: *mut c_void,
    description: [u16; 128],
}
#[link(name = "dxva2")]
unsafe extern "system" {
    fn GetNumberOfPhysicalMonitorsFromHMONITOR(monitor: *mut c_void, count: *mut u32) -> i32;
    fn GetPhysicalMonitorsFromHMONITOR(monitor: *mut c_void, count: u32, out: *mut PhysicalMonitor) -> i32;
    fn DestroyPhysicalMonitors(count: u32, monitors: *const PhysicalMonitor) -> i32;
    fn GetVCPFeatureAndVCPFeatureReply(
        monitor: *mut c_void,
        code: u8,
        kind: *mut u32,
        current: *mut u32,
        maximum: *mut u32,
    ) -> i32;
    fn SetVCPFeature(monitor: *mut c_void, code: u8, value: u32) -> i32;
}
struct Dpi(DPI_AWARENESS_CONTEXT);
impl Drop for Dpi {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: restore the context obtained from this thread at entry.
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
struct Monitor(PhysicalMonitor);
impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: exactly one handle returned by GetPhysicalMonitorsFromHMONITOR.
        unsafe {
            DestroyPhysicalMonitors(1, &self.0);
        }
    }
}
fn open(key: &str) -> Result<Monitor, String> {
    let displays = oink_display::platform_backend()
        .map_err(|e| e.to_string())?
        .displays()
        .map_err(|e| e.to_string())?;
    let target = displays
        .iter()
        .find(|d| d.key == key)
        .ok_or("The selected controller display is not connected")?;
    if displays.iter().filter(|d| d.bounds == target.bounds).count() != 1 {
        return Err("Mirrored display identities are ambiguous; use an extended desktop".into());
    }
    let b = target.bounds;
    let rect = RECT {
        left: b.x as i32,
        top: b.y as i32,
        right: b.right() as i32,
        bottom: b.bottom() as i32,
    };
    // SAFETY: predefined context; its previous value is restored by Dpi.
    let _dpi = Dpi(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) });
    // SAFETY: rect and info are initialized and valid for the synchronous calls.
    let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: monitor came from the native monitor lookup; info has the correct size.
    if monitor.0.is_null()
        || !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool()
        || info.rcMonitor.left != rect.left
        || info.rcMonitor.top != rect.top
        || info.rcMonitor.right != rect.right
        || info.rcMonitor.bottom != rect.bottom
    {
        return Err("Native display geometry changed; refresh the monitor bindings".into());
    }
    let mut count = 0;
    // SAFETY: monitor is live and count is a valid output pointer.
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor.0, &mut count) } == 0 || count != 1 {
        return Err("This display must resolve to exactly one physical DDC monitor".into());
    }
    let mut physical = PhysicalMonitor {
        handle: std::ptr::null_mut(),
        description: [0; 128],
    };
    // SAFETY: the preceding count was exactly one and the output array has one entry.
    if unsafe { GetPhysicalMonitorsFromHMONITOR(monitor.0, 1, &mut physical) } == 0 {
        return Err("Cannot open the physical monitor for DDC".into());
    }
    Ok(Monitor(physical))
}
pub(super) fn read(key: &str) -> Result<u16, String> {
    let monitor = open(key)?;
    let (mut current, mut maximum) = (0, 0);
    // SAFETY: owned live handle; VCP 0x60 is the standard input-source selector.
    if unsafe {
        GetVCPFeatureAndVCPFeatureReply(monitor.0.handle, 0x60, std::ptr::null_mut(), &mut current, &mut maximum)
    } == 0
    {
        return Err("The monitor did not report its current DDC input".into());
    }
    u16::try_from(current)
        .ok()
        .filter(|v| *v != 0)
        .ok_or_else(|| "The monitor returned an invalid input value".into())
}
pub(super) fn write_only(key: &str, value: u16) -> Result<(), String> {
    let monitor = open(key)?;
    // SAFETY: owned live handle; the user configured this nonzero input value.
    if value == 0 || unsafe { SetVCPFeature(monitor.0.handle, 0x60, u32::from(value)) } == 0 {
        return Err("The monitor rejected the DDC input switch".into());
    }
    Ok(())
}
pub(super) fn write(key: &str, value: u16) -> Result<(), String> {
    write_only(key, value)?;
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        if read(key).is_ok_and(|actual| actual == value) {
            return Ok(());
        }
    }
    Err(format!("Input {value} was not confirmed by the monitor"))
}
