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

Runtime validation on Linux `6.12.101-1-MANJARO` confirmed approximately
2500–2800 RPM on channel 0. Channel 1 was readable but remained at 0 RPM during
idle and a short CPU load, so it may be an unused firmware channel or a stopped
second fan.

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

## Optional DKMS installation

After validating the temporary module, install the source for automatic rebuilds
on kernel updates:

```bash
sudo install -d /usr/src/honor-fmi-hwmon-0.1.0
sudo install -m 644 Makefile dkms.conf honor-fmi-hwmon.c \
  /usr/src/honor-fmi-hwmon-0.1.0/
sudo dkms add honor-fmi-hwmon/0.1.0
sudo dkms install honor-fmi-hwmon/0.1.0
echo honor_fmi_hwmon | sudo tee /etc/modules-load.d/honor-fmi-hwmon.conf
```

Remove it completely with:

```bash
sudo rm -f /etc/modules-load.d/honor-fmi-hwmon.conf
sudo dkms remove honor-fmi-hwmon/0.1.0 --all
sudo rm -rf /usr/src/honor-fmi-hwmon-0.1.0
```

## Upstream direction

Before submission, confirm both channels at idle and under automatic fan ramp,
test repeated reads and suspend/resume, and ask the platform-x86 and hwmon
maintainers whether this belongs in `drivers/hwmon/` or under
`drivers/platform/x86/`. The first patch should remain monitoring-only.
