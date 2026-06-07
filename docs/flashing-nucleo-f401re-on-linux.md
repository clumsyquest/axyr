
# Flashing the NUCLEO-F401RE from Linux

Building and flashing this board with Zephyr on Linux, plus the two gotchas
that can make the ST-LINK look broken when it isn't.

## Environment

- **Board:** NUCLEO-F401RE (STM32F401xE, Cortex-M4, chip ID `0x433`, 512 KB flash)
- **Host:** Ubuntu on an AMD laptop (AMD xHCI USB controller)
- **Tools:** Zephyr + `west`, `stlink-tools` (`st-info`, `st-flash`)

## Quick start (verified)

```bash
# 1. Build — compile only, nothing reaches the chip yet
west build -p always -b nucleo_f401re samples/hello_world

# 2. Prime the SWD link under reset, THEN flash.
#    west flash alone fails on a chip that is already running: Zephyr idles in WFI,
#    and the probe cannot hot-attach a sleeping core. st-info --connect-under-reset
#    resets the chip via NRST and leaves the link grabbable, so west flash then works.
st-info --probe --connect-under-reset      # -> chipid: 0x433
west flash

# 3. Read the serial output
screen /dev/ttyACM0 115200                 # quit: Ctrl+A, then K, then Y
```

`west build` only compiles (`build/zephyr/zephyr.bin`); `west flash` is what writes it.

### Cleaner: make west flash connect under reset by itself

The two-step prime can be folded into one command by injecting the connect-under-reset
config into the OpenOCD runner:

```bash
west flash --cmd-pre-init "reset_config srst_only srst_nogate connect_assert_srst"
```

If this works on your setup, the separate `st-info` prime is no longer needed. It can
be made permanent via a local `openocd.cfg`.

## Gotcha 1 — SWD can't hot-attach a running (WFI) core

**Symptom:** on a chip already running firmware, `west flash` fails to connect, and
`st-info --probe` returns `chipid: 0x000` / "Unable to get core ID". The ST-LINK COM
LED is orange instead of green. The probe looks dead but is fine.

**Cause:** the running Zephyr app idles in low-power `WFI`. A normal "hot" attach to a
sleeping core is unreliable. A *blank* chip attaches fine hot — which is why a fresh
board's first flash works, then later attaches fail.

**Fix:** connect under reset (assert NRST before connecting), via any of:
- `st-info --probe --connect-under-reset` (prime, then `west flash`)
- `st-flash --connect-under-reset --reset write build/zephyr/zephyr.bin 0x08000000`
- `west flash --cmd-pre-init "reset_config srst_only srst_nogate connect_assert_srst"`

## Gotcha 2 — USB enumeration on AMD xHCI (`error -71`)

**Symptom:** `dmesg` shows `can't set config #1, error -71`; `lsusb -t` shows the
ST-LINK bare with no interfaces; `st-info` reports `Found 0 stlink programmers`;
`west flash` fails with "claim interface failed".

**Cause:** a quirk of the AMD xHCI host controller with this full-speed device. The
probe enumerates but `SET_CONFIGURATION` fails, so no interfaces come up.

**Mitigations (intermittent):**
- GRUB kernel parameters: `amd_iommu=off usbcore.old_scheme_first=1`
- Re-bind the controller: `sudo resetusb` (unbind/rebind `xhci_hcd`)
- A full reboot resets enumeration most reliably

**Deterministic fix:** a **USB 2.0 hub** between laptop and probe. Its Transaction
Translator regenerates clean signaling and isolates the device from the quirk.

## Appendix — one-time host setup

```bash
# Allow non-root access to the ST-LINK
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0483", ATTRS{idProduct}=="374b", MODE="0666"' \
  | sudo tee /etc/udev/rules.d/49-stlink.rules
sudo udevadm control --reload-rules && sudo udevadm trigger

# Free the USB/serial interfaces from interfering services
sudo apt remove brltty
sudo systemctl stop ModemManager
```
