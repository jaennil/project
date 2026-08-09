# Linux upstream RFC

The patch in this directory is based on Linux master commit `b9b3e33b70b7`
and adds a read-only hwmon driver for the HONOR FMI-XX.

## Validation

- built against Linux `6.12.101-1-MANJARO` headers;
- strict `scripts/checkpatch.pl`: 0 errors, 0 warnings, 0 checks;
- tested on HONOR FMI-XX, BIOS 1.09 dated 2025-01-14;
- channel 0 observed around 2500–2800 RPM during idle and a short CPU load;
- channel 1 was readable and remained at 0 RPM;
- repeated reads work through `sensors`, bbtop, Prometheus and Grafana;
- the driver exposes no write or PWM operation.

Suspend/resume and a second cold boot should be tested before changing the RFC
to a non-RFC patch.

## Proposed recipients

To:

- Guenter Roeck `<linux@roeck-us.net>`
- `linux-hwmon@vger.kernel.org`

Cc for placement review:

- Hans de Goede `<hansg@kernel.org>`
- Ilpo Järvinen `<ilpo.jarvinen@linux.intel.com>`
- `platform-driver-x86@vger.kernel.org`
- `linux-kernel@vger.kernel.org`

The RFC should be sent with plain-text email using `git send-email`. Do not send
the patch through a GitHub pull request.
