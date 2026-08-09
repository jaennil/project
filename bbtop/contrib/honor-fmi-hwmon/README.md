# HONOR FMI-XX fan monitoring prototype

This experimental, read-only Linux hwmon driver exposes both fan speeds of the
HONOR FMI-XX as `fan1_input` and `fan2_input` in RPM.

The machine's ACPI DSDT provides a serialized `\GFNS` method. Its input buffer
selects fan 0 or fan 1, and its output contains a status byte followed by a
little-endian 16-bit RPM value. The firmware reads these values from EC
registers `0x0A00`–`0x0A03`. This module calls `\GFNS`; it does not access the EC
directly and contains no write or PWM operation.

## Compatibility

The module has an exact DMI match for:

- system vendor: `HONOR`
- product name: `FMI-XX`
- tested firmware: `1.09`, dated `2025-01-14`

Do not broaden the DMI match until the same ACPI contract is confirmed on another
model.

## Build and test

Build against the running kernel:

```bash
make
```

Load temporarily and inspect the resulting sensor. Loading requires root, but
reading the RPM attributes normally does not:

```bash
sudo insmod honor-fmi-hwmon.ko
sensors honor_fmi-virtual-0
grep . /sys/class/hwmon/hwmon*/fan*_input
sudo rmmod honor_fmi_hwmon
```

Before loading, check that `uname -r` matches the headers used for the build.
The module is intentionally not installed persistently while it is experimental.

## Upstream direction

Before submission, confirm both channels at idle and under automatic fan ramp,
test repeated reads and suspend/resume, and ask the platform-x86 and hwmon
maintainers whether this belongs in `drivers/hwmon/` or under
`drivers/platform/x86/`. The first patch should remain monitoring-only.

