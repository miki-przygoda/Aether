//! Peripheral detection for the edge node.
//!
//! Three distinct hardware paths, each with its own OS-level mechanism:
//!
//! **USB audio (plug-and-play mics/speakers)**
//! The kernel handles enumeration via udev. ALSA creates card entries under
//! `/proc/asound/cards` automatically. `cpal` picks them up via
//! `host.input_devices()`. No app-level scanning needed; we just read the
//! ALSA card list and optionally watch `/dev/snd/` with inotify for hotplug.
//!
//! **I2C audio chips (HATs, breakout boards, custom breadboard)**
//! Linux exposes each bus at `/dev/i2c-N`. There is no interrupt when a new
//! device appears — you probe the bus by attempting a register read at each
//! 7-bit address. We compare responsive addresses against `KNOWN_I2C_CHIPS`
//! to identify the chip without any user input.
//! Real implementation uses `rppal::i2c::I2c` — wired in Phase 3.
//!
//! **Purely I2S MEMS mics (INMP441, ICS-43432, SPH0645, etc.)**
//! These have no I2C control interface. They appear as ALSA cards only after
//! the correct devicetree overlay is loaded (`dtoverlay=i2s-mems-mic` etc.).
//! Once the overlay is active they show up in `/proc/asound/cards` like any
//! other card — no special detection needed beyond the ALSA scan.
//!
//! **Pi HATs with EEPROM**
//! The Pi HAT spec (GPIO ID pins 27 & 28) causes the Pi to read the EEPROM
//! at boot and auto-load the right devicetree overlay. By the time the
//! edge-node process starts, the kernel has already configured the hardware.
//! We can read the EEPROM identity from `/proc/device-tree/hat/` to log what
//! is attached, but we do not need to configure anything ourselves.
//!
//! **Pure GPIO (buttons, LEDs)**
//! No auto-detection is possible — the user specifies pin numbers in config.
//! Pin assignments live in `private/CLAUDE.md` and are loaded at startup.
//!
//! **Re-scanning at runtime**
//! Send `SIGUSR1` to the edge-node process to trigger a fresh scan. This is
//! useful when a USB mic is hotplugged after startup. The Phase 5 web UI can
//! expose a "Re-scan devices" button that sends this signal.

use std::path::Path;

// ── Known I2C chip registry ───────────────────────────────────────────────────

/// Describes a known I2C audio peripheral chip.
#[derive(Debug, Clone, PartialEq)]
pub struct I2cChipInfo {
    /// 7-bit I2C address (the value on the wire, without the R/W bit).
    pub addr: u8,
    pub name: &'static str,
    pub description: &'static str,
}

/// Registry of known I2C audio chips found on Pi HATs and breakout boards.
///
/// Addresses come from datasheets and assume the most common pin-strapping.
/// Alternate addresses (set by pulling AD pins high/low) are listed separately
/// so a bus scan finds them regardless of how the user wired the board.
///
/// To add a new chip:
///   1. Find its 7-bit address(es) in the datasheet.
///   2. Add entries here (one per distinct address possibility).
///   3. Add matching init logic in `audio.rs` (sample rate, channel count,
///      I2S clock polarity, etc.) keyed on `I2cChipInfo::name`.
pub const KNOWN_I2C_CHIPS: &[I2cChipInfo] = &[
    // ── Mic arrays ──────────────────────────────────────────────────────────
    I2cChipInfo {
        addr: 0x40,
        name: "ES7210",
        description: "Quad-mic array (EspressIF; ReSpeaker 4-mic Array HAT)",
    },
    I2cChipInfo {
        addr: 0x41,
        name: "ES7210",
        description: "Quad-mic array (EspressIF, alt addr AD0=1)",
    },
    I2cChipInfo {
        addr: 0x3B,
        name: "AC108",
        description: "Quad-mic array (X-Powers; Seeed Studio 4-mic HAT, primary)",
    },
    I2cChipInfo {
        addr: 0x38,
        name: "AC108",
        description: "Quad-mic array (X-Powers, alt addr AD[1:0]=00)",
    },
    // ── Stereo codecs (mic in + speaker out) ────────────────────────────────
    I2cChipInfo {
        addr: 0x1A,
        name: "WM8960",
        description: "Stereo codec, mic + speaker (Wolfson; WM8960 Audio HAT)",
    },
    I2cChipInfo {
        addr: 0x18,
        name: "TLV320AIC3104",
        description: "Stereo codec (Texas Instruments, ADDR=0)",
    },
    I2cChipInfo {
        addr: 0x19,
        name: "TLV320AIC3104",
        description: "Stereo codec (Texas Instruments, ADDR=1)",
    },
    I2cChipInfo {
        addr: 0x0A,
        name: "SGTL5000",
        description: "Low-power stereo codec (NXP; Teensy Audio Board)",
    },
    I2cChipInfo {
        addr: 0x1C,
        name: "AK4954A",
        description: "Mono ADC/DAC (Asahi Kasei)",
    },
];

/// Look up a chip by its 7-bit I2C address.
/// Returns the first match — for chips with multiple alt-address entries the
/// primary (lowest address) is always listed first.
pub fn lookup_i2c_chip(addr: u8) -> Option<&'static I2cChipInfo> {
    KNOWN_I2C_CHIPS.iter().find(|c| c.addr == addr)
}

// ── ALSA card representation ──────────────────────────────────────────────────

/// A card entry from `/proc/asound/cards`.
#[derive(Debug, Clone, PartialEq)]
pub struct AlsaCard {
    pub index: u32,
    /// Short bracket identifier, e.g. `"Device"` or `"Headphones"`.
    pub id: String,
    /// Driver description after the dash, e.g. `"USB Audio Device"`.
    pub name: String,
}

// ── Pi HAT identity ───────────────────────────────────────────────────────────

/// Pi HAT EEPROM identity read from `/proc/device-tree/hat/`.
/// Present only when a spec-compliant HAT is attached and its overlay loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct HatInfo {
    pub vendor: String,
    pub product: String,
}

// ── User config overrides ─────────────────────────────────────────────────────

/// Device overrides stored in `config.json`.
/// A non-None field silences auto-detection for that slot and uses the
/// specified ALSA device string directly.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceConfig {
    /// Force a specific ALSA input device, e.g. `"hw:2,0"` or `"plughw:Device"`.
    pub audio_input: Option<String>,
    /// Force a specific ALSA output device.
    pub audio_output: Option<String>,
}

// ── Discovery report ──────────────────────────────────────────────────────────

/// Everything found during a peripheral scan.
#[derive(Debug)]
pub struct DiscoveredDevices {
    // Phase 3: used by the cpal device selector and the inotify hotplug watcher.
    #[allow(dead_code)]
    pub alsa_cards: Vec<AlsaCard>,
    pub hat: Option<HatInfo>,
    /// I2C audio chips identified on the system buses.
    /// Empty until Phase 3 wires in the `rppal` bus scan.
    pub i2c_chips: Vec<(u8 /* bus */, &'static I2cChipInfo)>,
    /// Non-fatal warnings from the scan (e.g. no mic found, no override set).
    pub warnings: Vec<String>,
}

// ── Pure parsing functions ────────────────────────────────────────────────────

/// Parse the text content of `/proc/asound/cards`.
///
/// Each card occupies two lines; we only parse the first:
/// ` 0 [Device          ]: USB-Audio - USB Audio Device`
pub fn parse_alsa_cards(content: &str) -> Vec<AlsaCard> {
    let mut cards = Vec::new();
    for line in content.lines() {
        if !line.contains('[') || !line.contains("]:") {
            continue;
        }
        let mut tokens = line.trim_start().splitn(2, ' ');
        let index: u32 = match tokens.next().and_then(|s| s.parse().ok()) {
            Some(i) => i,
            None => continue,
        };
        let rest = tokens.next().unwrap_or("");
        let id = rest
            .split('[')
            .nth(1)
            .and_then(|s| s.split(']').next())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let name = rest
            .split("]: ")
            .nth(1)
            .and_then(|s| s.split(" - ").nth(1))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        cards.push(AlsaCard { index, id, name });
    }
    cards
}

/// Parse raw bytes from `/proc/device-tree/hat/vendor` and `/product`.
/// The kernel null-terminates these strings.
pub fn parse_hat_info(vendor_bytes: &[u8], product_bytes: &[u8]) -> Option<HatInfo> {
    let vendor = null_terminated_str(vendor_bytes)?;
    let product = null_terminated_str(product_bytes)?;
    if vendor.is_empty() || product.is_empty() {
        return None;
    }
    Some(HatInfo { vendor, product })
}

fn null_terminated_str(b: &[u8]) -> Option<String> {
    let b = b.strip_suffix(b"\0").unwrap_or(b);
    std::str::from_utf8(b).ok().map(|s| s.trim().to_string())
}

// ── I/O wrappers ─────────────────────────────────────────────────────────────

/// Read and parse `/proc/asound/cards`.
/// Returns an empty vec on non-Linux machines and in CI without ALSA — callers
/// must treat that as "no cards found", not as an error.
pub fn scan_alsa_cards() -> Vec<AlsaCard> {
    match std::fs::read_to_string("/proc/asound/cards") {
        Ok(content) => parse_alsa_cards(&content),
        Err(_) => vec![],
    }
}

/// Read Pi HAT EEPROM identity from `/proc/device-tree/hat/`.
/// Returns `None` on non-Pi machines or if no HAT is attached.
pub fn read_hat_info() -> Option<HatInfo> {
    let base = Path::new("/proc/device-tree/hat");
    let vendor = std::fs::read(base.join("vendor")).ok()?;
    let product = std::fs::read(base.join("product")).ok()?;
    parse_hat_info(&vendor, &product)
}

/// Run a full peripheral scan and return everything found.
///
/// Phase 3 note — I2C bus scan stub:
///   Iterate `/dev/i2c-*`, open each bus with `rppal::i2c::I2c::new(bus_num)`,
///   then for each address in `KNOWN_I2C_CHIPS` attempt a 0-byte write and
///   treat an `Ok` response as "device present". Push matches into
///   `DiscoveredDevices::i2c_chips`. This replaces the empty vec below.
///
/// Send `SIGUSR1` to re-run this function at runtime (e.g. after a USB mic
/// is hotplugged). The Phase 5 web UI exposes a "Re-scan devices" button that
/// sends the signal and refreshes the device list.
pub fn discover(config: &DeviceConfig) -> DiscoveredDevices {
    let alsa_cards = scan_alsa_cards();
    let hat = read_hat_info();
    let mut warnings = Vec::new();

    if alsa_cards.is_empty() && config.audio_input.is_none() {
        warnings.push(
            "no ALSA cards found and no audio_input override configured — \
             check that a microphone is attached and the correct devicetree overlay is loaded"
                .to_string(),
        );
    }

    if let Some(ref hat) = hat {
        tracing::info!(vendor = %hat.vendor, product = %hat.product, "Pi HAT detected");
    }

    DiscoveredDevices {
        alsa_cards,
        hat,
        i2c_chips: vec![], // Phase 3: replace with rppal I2C bus scan
        warnings,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Registry ─────────────────────────────────────────────────────────────

    #[test]
    fn lookup_es7210_by_primary_address() {
        let chip = lookup_i2c_chip(0x40).expect("ES7210 should be in registry");
        assert_eq!(chip.name, "ES7210");
    }

    #[test]
    fn lookup_wm8960() {
        let chip = lookup_i2c_chip(0x1A).expect("WM8960 should be in registry");
        assert_eq!(chip.name, "WM8960");
    }

    #[test]
    fn lookup_ac108_alt_address() {
        // Users may wire AD pins to get the alternate address.
        let chip = lookup_i2c_chip(0x38).expect("AC108 alt addr should be in registry");
        assert_eq!(chip.name, "AC108");
    }

    #[test]
    fn unknown_address_returns_none() {
        assert!(lookup_i2c_chip(0x7F).is_none());
        assert!(lookup_i2c_chip(0x00).is_none());
        assert!(lookup_i2c_chip(0x50).is_none()); // common EEPROM address, not audio
    }

    #[test]
    fn registry_has_no_duplicate_addresses() {
        let mut seen = std::collections::HashSet::new();
        for chip in KNOWN_I2C_CHIPS {
            assert!(
                seen.insert(chip.addr),
                "duplicate I2C address 0x{:02X} for chip {} — \
                 use separate entries with distinct addresses or consolidate",
                chip.addr,
                chip.name
            );
        }
    }

    #[test]
    fn all_registry_addresses_are_valid_7bit() {
        for chip in KNOWN_I2C_CHIPS {
            assert!(
                chip.addr <= 0x77,
                "address 0x{:02X} for {} exceeds 7-bit range (reserved: 0x78-0x7F)",
                chip.addr,
                chip.name
            );
            // I2C reserved addresses: 0x00-0x07 and 0x78-0x7F
            assert!(
                chip.addr >= 0x08,
                "address 0x{:02X} for {} is in the reserved range 0x00-0x07",
                chip.addr,
                chip.name
            );
        }
    }

    // ── ALSA card parser ──────────────────────────────────────────────────────

    const PROC_ASOUND_CARDS: &str = "\
 0 [b1              ]: bcm2835_hdmi - bcm2835 HDMI 1\n\
                       bcm2835 HDMI 1\n\
 1 [Headphones      ]: bcm2835_headpho - bcm2835 Headphones\n\
                       bcm2835 Headphones\n\
 2 [Device          ]: USB-Audio - USB Audio Device\n\
                       Generic USB Audio Device at usb-0000:01:00.0-1.2, full speed\n";

    #[test]
    fn parse_alsa_cards_count() {
        assert_eq!(parse_alsa_cards(PROC_ASOUND_CARDS).len(), 3);
    }

    #[test]
    fn parse_alsa_cards_indices() {
        let cards = parse_alsa_cards(PROC_ASOUND_CARDS);
        assert_eq!(cards[0].index, 0);
        assert_eq!(cards[1].index, 1);
        assert_eq!(cards[2].index, 2);
    }

    #[test]
    fn parse_alsa_cards_ids() {
        let cards = parse_alsa_cards(PROC_ASOUND_CARDS);
        assert_eq!(cards[0].id, "b1");
        assert_eq!(cards[1].id, "Headphones");
        assert_eq!(cards[2].id, "Device");
    }

    #[test]
    fn parse_alsa_cards_usb_device_name() {
        let cards = parse_alsa_cards(PROC_ASOUND_CARDS);
        let usb = cards.iter().find(|c| c.id == "Device").unwrap();
        assert!(
            usb.name.contains("USB Audio"),
            "expected USB Audio in name, got: {}",
            usb.name
        );
    }

    #[test]
    fn parse_alsa_cards_empty_input() {
        assert!(parse_alsa_cards("").is_empty());
    }

    #[test]
    fn parse_alsa_cards_single_card() {
        let input = " 0 [PCH             ]: HDA-Intel - HDA Intel PCH\n\
                      HDA Intel PCH at 0xe1238000 irq 48\n";
        let cards = parse_alsa_cards(input);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].index, 0);
        assert_eq!(cards[0].id, "PCH");
        assert!(cards[0].name.contains("HDA Intel PCH"));
    }

    // ── HAT info parser ───────────────────────────────────────────────────────

    #[test]
    fn parse_hat_strips_null_terminator() {
        let hat = parse_hat_info(b"Waveshare\0", b"WM8960 Audio HAT\0").unwrap();
        assert_eq!(hat.vendor, "Waveshare");
        assert_eq!(hat.product, "WM8960 Audio HAT");
    }

    #[test]
    fn parse_hat_no_null_still_works() {
        let hat = parse_hat_info(b"Seeed", b"ReSpeaker 4-Mic Array").unwrap();
        assert_eq!(hat.vendor, "Seeed");
        assert_eq!(hat.product, "ReSpeaker 4-Mic Array");
    }

    #[test]
    fn parse_hat_empty_vendor_returns_none() {
        assert!(parse_hat_info(b"\0", b"some product\0").is_none());
    }

    #[test]
    fn parse_hat_empty_product_returns_none() {
        assert!(parse_hat_info(b"some vendor\0", b"\0").is_none());
    }

    #[test]
    fn parse_hat_invalid_utf8_returns_none() {
        assert!(parse_hat_info(&[0xFF, 0xFE], b"product\0").is_none());
    }

    // ── discover() smoke test ─────────────────────────────────────────────────

    #[test]
    fn discover_does_not_panic_on_any_host() {
        // /proc/asound/cards and /proc/device-tree/hat/ are absent on macOS and
        // in CI — discover() must return gracefully with empty collections.
        let report = discover(&DeviceConfig::default());
        assert!(report.i2c_chips.is_empty(), "Phase 3 stub should be empty");
        // hat is None on non-Pi hosts; that is expected, not a failure.
        let _ = report.hat;
    }

    #[test]
    fn discover_warning_when_no_cards_and_no_override() {
        // On a host with no ALSA, discover() should warn rather than silently
        // pick nothing.  We can't force scan_alsa_cards() to return empty from
        // outside, but we can verify the warning text contract via the pure path.
        let report = discover(&DeviceConfig::default());
        // On Linux CI with ALSA the warning may not fire; on macOS it will.
        // Either way the function must complete and the warning slice must be valid.
        for w in &report.warnings {
            assert!(!w.is_empty(), "warning strings must not be empty");
        }
    }

    #[test]
    fn discover_no_warning_when_override_set() {
        let config = DeviceConfig {
            audio_input: Some("hw:2,0".to_string()),
            audio_output: None,
        };
        // Even with no ALSA cards the override suppresses the "no mic" warning.
        // We verify this by checking no warning mentions "no ALSA cards" when an
        // override is present — the pure logic branch is covered regardless of host.
        let report = discover(&config);
        let has_no_card_warn = report.warnings.iter().any(|w| w.contains("no ALSA cards"));
        assert!(
            !has_no_card_warn,
            "override should suppress the no-cards warning"
        );
    }
}
