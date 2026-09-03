# mouse testing suite

Measures what a pointing device actually does, rather than what it claims.

The interface is deliberately plain: black, grey and white, monospace, no
animation. Colour appears only to mean pass, warning or fail.

## Approach: one codebase, two backends

One Rust binary, built from one source tree, with the platform-specific
measurement layers selected at compile time. Neither platform is degraded to
keep them uniform, because the capability difference is real and the program
models it explicitly rather than papering over it.

Every measurement is taken at up to three levels, and the difference between
them is the point:

| level | macOS | Windows |
|---|---|---|
| **device** — reports as the OS receives them | `IOHIDDevice` input reports, timestamped by the driver | Raw Input (`WM_INPUT`), timestamped on arrival |
| **system** — after the OS turns reports into input events | system-wide mouse event stream | `WH_MOUSE_LL` low-level hook |
| **application** — what an ordinary program receives | events delivered to this window | events delivered to this window |

Two consequences are stated in the interface rather than hidden:

- The **device** level is per-device. The **system** level is not, on either
  platform: neither OS attributes a physical device to a system input event.
  With two pointing devices moving at once, the system level shows their sum.
- The device level is the rate *the OS receives*, not the rate the mouse sends.
  No unprivileged path exists below the OS class driver on either platform.
  Getting lower needs a signed kernel driver, which is a different product.

## Build

Needs Rust 1.92 or newer.

```
cargo build --release
```

### macOS (Apple Silicon)

```
./scripts/make-app-bundle.sh
open target/mouse-testing.app
```

Use the bundle rather than `cargo run`. macOS attaches permission grants to the
"responsible process", which for a binary started from a terminal is the
terminal or editor, not the binary; the bundle launched with `open` is its own
responsible process and gets its own entry in System Settings.

### Windows 10/11

```
cargo build --release
.\target\release\mouse-testing.exe
```

Build natively for the machine: `x86_64-pc-windows-msvc` on an Intel or AMD PC,
`aarch64-pc-windows-msvc` on an ARM PC. Do not ship or run a 32-bit build. The
app detects and warns about x64-on-ARM64 emulation, because emulated code adds
translation overhead that lands in the middle of every interval measurement.

## Permissions

No administrator or root privilege is required anywhere, and none is requested.

### macOS: Input Monitoring

Required for the **device** level only. Everything else, including the full
device list, identifiers, report descriptor and the host environment report,
works with no grant at all.

macOS gates Generic Desktop Mouse and Keyboard HID collections behind Input
Monitoring. Without it, `IOHIDDeviceOpen` returns `kIOReturnNotPermitted`. The
app detects this and says so with the exact error; it never reports zero.

To grant it:

1. **System Settings > Privacy & Security > Input Monitoring**
2. Switch on **mouse testing suite** (use **+** and pick
   `target/mouse-testing.app` if it is not listed)
3. **Quit and reopen the app.** macOS does not apply the grant to an
   already-running process.

The app has a button that opens that pane directly.

Because the bundle is ad-hoc signed rather than Developer ID signed, macOS
partly identifies it by its code hash, so rebuilding can require re-granting.
That is a property of unsigned software, not a defect in the app.

### Windows: none

Raw Input and low-level mouse hooks need no privilege and no permission grant.

## Command line

```
mouse-testing               run the interface
mouse-testing --dump        print the device and environment report, then exit
mouse-testing --screenshot FILE
                            render the window to a PNG and exit
```

`--dump` is useful over SSH and for bug reports. `--screenshot` captures from
inside the process, so it needs no Screen Recording permission.

## What has been verified, and how

Honesty about this matters more than a green checkmark.

- **macOS**: built and run on macOS 15.6.1, Apple Silicon (M4). Device
  enumeration, identifiers, IORegistry topology, permission probing and the
  environment report are verified against real hardware.
- **Windows**: type-checked for both `x86_64-pc-windows-msvc` and
  `aarch64-pc-windows-msvc`, including compile-time assertions on the Win32
  struct layouts the decoder depends on. It has **not been run on Windows**;
  no Windows machine was available. Treat the Windows path as unproven at
  runtime until you exercise it.
- **Interface**: layout invariants are enforced by tests that run headlessly
  (`cargo test`), including that a numeric readout's width does not change with
  its value, that every text style is monospace, and that nothing is rounded.

## Non-goals

No RGB control, macro recording, writing configuration to the mouse, cloud
features, accounts, or auto-update. Measurement only.
