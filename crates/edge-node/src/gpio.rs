use aether_core::NodeState;

// ── LED pattern (platform-independent) ───────────────────────────────────────

/// Visual output pattern for the 3-colour status LED.
#[cfg_attr(not(feature = "gpio"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedPattern {
    Off,
    /// Solid green — node is idle and ready.
    SolidGreen,
    /// Pulsing blue — node is actively listening or processing speech.
    PulseBlue,
    /// Flashing red — error condition or Do-Not-Disturb mode.
    FlashRed,
}

/// Map a `NodeState` to the correct LED pattern.
/// This function is always compiled so unit tests run on every platform.
#[cfg_attr(not(feature = "gpio"), allow(dead_code))]
pub fn state_to_pattern(state: NodeState) -> LedPattern {
    match state {
        NodeState::Idle => LedPattern::SolidGreen,
        NodeState::Listening => LedPattern::PulseBlue,
        NodeState::Processing => LedPattern::PulseBlue,
        NodeState::Error => LedPattern::FlashRed,
    }
}

// ── Hardware LED driver (Raspberry Pi only) ───────────────────────────────────

/// Drive a common-cathode RGB LED connected to three GPIO output pins.
///
/// Pin assignments are documented in `private/CLAUDE.md` and loaded at runtime
/// from the `AETHER_LED_RED_PIN`, `AETHER_LED_GREEN_PIN`, `AETHER_LED_BLUE_PIN`
/// environment variables.
///
/// Pulsing and flashing are driven by background tokio tasks rather than
/// blocking PWM so the caller never blocks.
#[cfg(feature = "gpio")]
pub struct LedController {
    red: rppal::gpio::OutputPin,
    green: rppal::gpio::OutputPin,
    blue: rppal::gpio::OutputPin,
}

#[cfg(feature = "gpio")]
impl LedController {
    /// Open the GPIO pins for LED control.
    pub fn new(red_pin: u8, green_pin: u8, blue_pin: u8) -> anyhow::Result<Self> {
        let gpio = rppal::gpio::Gpio::new()?;
        Ok(Self {
            red: gpio.get(red_pin)?.into_output(),
            green: gpio.get(green_pin)?.into_output(),
            blue: gpio.get(blue_pin)?.into_output(),
        })
    }

    /// Apply a static LED pattern immediately.
    ///
    /// For `PulseBlue` and `FlashRed` this sets the initial state only; callers
    /// must drive the time-varying component with a background task.
    pub fn apply(&mut self, pattern: LedPattern) {
        match pattern {
            LedPattern::Off => {
                self.red.set_low();
                self.green.set_low();
                self.blue.set_low();
            }
            LedPattern::SolidGreen => {
                self.red.set_low();
                self.green.set_high();
                self.blue.set_low();
            }
            LedPattern::PulseBlue => {
                self.red.set_low();
                self.green.set_low();
                self.blue.set_high();
            }
            LedPattern::FlashRed => {
                self.red.set_high();
                self.green.set_low();
                self.blue.set_low();
            }
        }
    }

    /// Read the GPIO env vars and construct the controller.
    /// Returns `Ok(None)` if any env var is unset, logging a warning.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let r = std::env::var("AETHER_LED_RED_PIN");
        let g = std::env::var("AETHER_LED_GREEN_PIN");
        let b = std::env::var("AETHER_LED_BLUE_PIN");
        match (r, g, b) {
            (Ok(r), Ok(g), Ok(b)) => {
                let r: u8 = r
                    .parse()
                    .map_err(|_| anyhow::anyhow!("AETHER_LED_RED_PIN is not a valid u8"))?;
                let g: u8 = g
                    .parse()
                    .map_err(|_| anyhow::anyhow!("AETHER_LED_GREEN_PIN is not a valid u8"))?;
                let b: u8 = b
                    .parse()
                    .map_err(|_| anyhow::anyhow!("AETHER_LED_BLUE_PIN is not a valid u8"))?;
                Ok(Some(Self::new(r, g, b)?))
            }
            _ => {
                tracing::warn!(
                    "LED pins not configured (AETHER_LED_RED/GREEN/BLUE_PIN) — LED disabled"
                );
                Ok(None)
            }
        }
    }
}

// ── Panic button (Raspberry Pi only) ─────────────────────────────────────────

/// Register a GPIO interrupt on the panic button pin that sends a `KillSignal`
/// when pressed.
///
/// Pin assignment comes from `AETHER_PANIC_BUTTON_PIN` env var.  If unset,
/// the panic button is disabled and this function returns `Ok(())`.
///
/// The interrupt task runs for the lifetime of the process — it is intentionally
/// not cancellable (a kill signal should always be deliverable).
#[cfg(feature = "gpio")]
pub fn register_panic_button(
    kill_tx: tokio::sync::broadcast::Sender<crate::kill_signal::KillSignal>,
) -> anyhow::Result<()> {
    use rppal::gpio::{Gpio, Trigger};

    let pin_str = match std::env::var("AETHER_PANIC_BUTTON_PIN") {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("AETHER_PANIC_BUTTON_PIN not set — panic button disabled");
            return Ok(());
        }
    };
    let pin_num: u8 = pin_str
        .parse()
        .map_err(|_| anyhow::anyhow!("AETHER_PANIC_BUTTON_PIN is not a valid u8"))?;

    let gpio = Gpio::new()?;
    // Pull-up so the button pulls to GND when pressed (falling edge).
    let mut pin = gpio.get(pin_num)?.into_input_pullup();

    pin.set_async_interrupt(Trigger::FallingEdge, move |_| {
        tracing::warn!("panic button pressed — broadcasting KillSignal");
        let _ = kill_tx.send(crate::kill_signal::KillSignal);
    })?;

    // Keep the pin alive for the process lifetime by leaking it.
    std::mem::forget(pin);
    tracing::info!(pin = pin_num, "panic button registered");
    Ok(())
}

// ── Tests (platform-independent) ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_maps_to_solid_green() {
        assert_eq!(state_to_pattern(NodeState::Idle), LedPattern::SolidGreen);
    }

    #[test]
    fn listening_maps_to_pulse_blue() {
        assert_eq!(
            state_to_pattern(NodeState::Listening),
            LedPattern::PulseBlue
        );
    }

    #[test]
    fn processing_maps_to_pulse_blue() {
        assert_eq!(
            state_to_pattern(NodeState::Processing),
            LedPattern::PulseBlue
        );
    }

    #[test]
    fn error_maps_to_flash_red() {
        assert_eq!(state_to_pattern(NodeState::Error), LedPattern::FlashRed);
    }

    #[test]
    fn all_node_states_have_a_pattern() {
        let states = [
            NodeState::Idle,
            NodeState::Listening,
            NodeState::Processing,
            NodeState::Error,
        ];
        for state in states {
            // Smoke test: must not panic.
            let _ = state_to_pattern(state);
        }
    }
}
