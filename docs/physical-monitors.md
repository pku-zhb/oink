# Shared physical monitors

The shared monitor workflow binds each physical panel to its per-computer display identity and video input. Users confirm the relationship once, then choose which computer appears on each panel. This branch targets two horizontally arranged external monitors and two computers while retaining a generic per-panel model.

## User workflow

Open Settings, Layout, Physical monitor ownership, then start shared monitor setup. Keep the participating computers online. The computer that starts setup coordinates the saved mapping; other paired computers do not have to participate.

The wizard checks the participating computers, identifies exact system displays, lets users correct their physical order, and calibrates input bindings. A calibration temporarily shows one selected computer, labels its target screen, and restores the original input before asking for confirmation. Imported bindings can be accepted explicitly when the user has already verified them in daily use. Raw input values and readback policy remain in advanced settings; standard port labels are candidates, not a claim about a particular monitor's firmware.

Two monitors and two computers produce four visual presets. Per-panel selectors remain usable when a larger configuration exceeds the preset limit. Presets do not change the set of managed monitors.

Compatible mode switches monitor inputs and Oink routing while retaining system desktop spaces. Complete mode also removes managed, invisible displays from each operating system and selects a visible main display. Complete mode fails before writing inputs if its initial native plan would remove the last physical display. Use compatible mode when a computer receives no panels and has no other physical display to keep.

Applied changes require confirmation within fifteen seconds. Any visible participant can keep or revert the pending operation, including when the coordinating computer has no visible assigned panel. Restore configuration before switching returns to the saved entry state, rather than only clearing Oink routing. If the coordinator is offline, local recovery restores only the requesting computer and reports that limitation.

The confirmation is global to the main window, not tied to an open settings page. A visible participant opens its main window and places the prompt on an assigned display using that computer's native coordinate units. Reverting returns the window to a previously visible display when available. A temporary always-on-top state is restored when the prompt ends. Windows foreground restrictions remain in force; the workflow does not borrow the backend's UIAccess foreground grant. These native placement behaviors still require actual desktop acceptance.

## Ownership and state

`physical_monitors/model.rs` defines stable physical panel identities, endpoint bindings, horizontal visible rectangles, and scoped native plans. System display keys remain owned by their computers and are not used as global physical identities. The mapping is persisted on participants, versioned at its fixed coordinator, and checked against the current paired group. Saving performs no hardware writes; a failed save compensates copies already written to peers.

`physical_monitors/experience.rs` owns setup, asynchronous operation phases, participant admission, input switching, confirmation and compensation. Queries do not take the native operation lock. Only participating peers must be online and support the workflow. Global presets and existing Oink layout preferences are left alone outside explicit operations.

The coordinator serializes operations. Each owner also checks the operation identity, expiry and local journal under a per-process gate and a cross-process file lock. Invalid, stale or concurrent commands cannot reuse a newer recovery point. Input writes are separated from readback polling, and successful operations have a cooling period. Native writes are never automatically retried as ordinary request retransmission.

Bindings and entry snapshots are independent of the live display list, so a temporarily disabled display is not forgotten. Switching from a layout with disabled displays reconnects the saved baseline before issuing input changes. Hardware without input readback uses an explicitly confirmed baseline and does not claim hardware-verified results. A routing change outside the retained session requires renewed baseline confirmation.

## Protocol and storage

The optional `oink.physical-monitors/2` capability carries boot-fenced requests and replies over authenticated member C0, using appended message IDs `0x0d35` and `0x0d36`. JSON application bodies are bounded by the wire DTO. Ordinary v1 operation does not require this capability. Existing protocol vectors are preserved; new optional vectors are appended with the existing compatible-fixture tool.

The local API uses the existing method:

```json
{
  "method": "physical_monitors",
  "params": {
    "command": {
      "action": "experience",
      "request": { "action": "status" }
    }
  }
}
```

Requests include inspection, exact-screen identification, input reads, setup save, calibration, apply, keep, revert, session restore and local recovery. Keep and revert bind to the specific pending operation. The original local API remains for migration and read-only diagnostics; it cannot bypass an active v2 transaction or independently clear a configured system desktop session.

`physical-monitors/mapping.json` holds shared bindings and retained selection. `entry-recovery.json` holds the configuration before physical mode; `recovery.json` holds the immediately preceding state of one unconfirmed operation. These are local application data, not research-archive files. Recovery data is bounded and private on Unix. It contains display state, not credentials.

## Independent recovery

The same installed executable starts a narrowly scoped recovery role before CLI, GUI or backend startup. The parent requires a readiness acknowledgement before making changes. The helper uses a private journal, a deadline and the same interprocess lock as owner commands; it survives loss of the settings UI and does not initialize networking or input capture. Its environment is restricted to ordinary operating-system session variables.

The helper restores the journal on expiry and reconciles routing through local IPC when available. Startup also resumes incomplete recovery or reconciles a completed helper receipt. Disarming removes the pending journal; an older helper cannot restore a newer operation. Entry state stays available until an explicit session restore. A failed restore remains visible and blocks ordinary switching until handled.

A helper cannot recover a crashed operating system or make an unresponsive GPU driver process display requests. It is not a substitute for driver stability. Native helper readiness, cancellation and process-lifetime behavior must be validated in actual Windows packaging as well as with the portable application tests.

## Native adapters

Windows snapshots explicit initialized CCD path and mode fields. No struct padding is serialized. Apply retains unmanaged targets, shifts the retained main source to the origin, validates the supplied configuration, then applies it without saving to the display database or allowing silent mode changes. Restore uses the saved path and mode data. Unsupported configurations fail into an explicit compatible-mode choice.

macOS uses BetterDisplay for logical connection management and main-display selection. Pro and connection-management support are required. Complete-mode preparation checks the adapter's connection-state query before input writes. Restoring reconnects remembered screens, restores the main display and native origins, then checks the result. A remembered UUID alone does not prove that a disconnected display is reachable. Intel support is outside Oink's existing platform scope; no private macOS display API was added.

Both adapters require hardware acceptance for their exact monitors, drivers, cables and display modes. Checks include 12 and 21, each possible main display, reconnecting a disabled panel, 11 and 22 in compatible mode, cancelling confirmation, participant loss, and preserving unrelated screens. Application-specific full-screen windows, Spaces and icon placement need separate observation; exact window-position restoration is not promised.

## Development and tests

Run the normal frontend build and tests. Focused Rust tests are available under `physical_monitors::`, including a real authenticated two-node transport fixture with fake display hardware. That fixture covers shared setup, save compensation, confirmation from the other participant, native failure, timeout, complete restoration, explicit compatible mode and cancellable calibration.

The development-only `?fixture=physical-monitors` route renders the production components with a simulated transport. It does not connect to the running Oink backend or change any physical display. `?language=en` selects English. This route is excluded from production startup.

Build the Windows binaries with the existing preview workflow. Local non-Windows type-checking of the CCD adapter is useful but does not replace a full Windows build. Hardware tests are deliberately separate from automated model, transport and UI tests.

## Display-driver timeouts

A black screen followed by `0x116 VIDEO_TDR_FAILURE` is an operating-system display-driver recovery failure. Preserve the System event log and local crash dump before changing configuration. Do not repeatedly switch monitors to reproduce it while the user needs the machine.

Compare input selection, link-retraining timing, readback and native desktop changes separately. Keep the driver and other settings fixed for each comparison. A driver module identified in the dump narrows the investigation but does not establish whether a particular application command, driver defect, virtual display stack or hardware condition triggered it.

Do not modify TDR registry keys as a product workaround. Microsoft reserves those controls for targeted driver testing. Native disconnection and a monitor power-off command are different operations; the adapter uses logical disconnection and never uses monitor power-off as an invisible-desktop implementation.

References: [Microsoft SetDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setdisplayconfig), [BetterDisplay CLI](https://betterdisplay.pro/guide/integration/cli-reference/), [BetterDisplay connection management](https://betterdisplay.pro/guide/interface-reference/app-menu/device-menu/header-and-connection/), [Microsoft 0x116](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x116---video-tdr-failure), [Microsoft TDR debugging settings](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/tdr-registry-keys).
