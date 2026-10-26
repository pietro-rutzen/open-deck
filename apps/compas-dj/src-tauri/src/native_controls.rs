//! Controller-originated mixer and transport commands resolved on the controller thread and sent
//! straight to the engine queue, so performance controls never round-trip through React/IPC.
use crate::EngineMsg;
use compas_audio::{DeckTelemetry, FilterMode, NUM_DECKS};
use std::sync::atomic::{AtomicU32, Ordering};

const EQ_BANDS: usize = 3;
const EQ_LOW: usize = 0;
const EQ_MID: usize = 1;
const EQ_HIGH: usize = 2;
const EQ_LIMIT_DB: f64 = 26.0;
const TRIM_MAX: f64 = compas_core::control::TRIM_MAX_GAIN;
const TEMPO_LIMIT_PERCENT: f64 = 100.0;
const FILTER_DEAD_ZONE: f64 = 0.02;

/// Last EQ sent to each deck. A controller knob moves one band, but the engine command carries all
/// three, so UI and controller writes share this store instead of overwriting each other's bands.
#[derive(Default)]
pub(crate) struct DeckEqState([[AtomicU32; EQ_BANDS]; NUM_DECKS]);

impl DeckEqState {
    pub(crate) fn record(&self, deck: usize, low: f32, mid: f32, high: f32) {
        if let Some(bands) = self.0.get(deck) {
            for (band, value) in bands.iter().zip([low, mid, high]) {
                band.store(value.to_bits(), Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn with_band(
        &self,
        deck: usize,
        band: usize,
        value: f32,
    ) -> Option<[f32; EQ_BANDS]> {
        let bands = self.0.get(deck)?;
        bands.get(band)?.store(value.to_bits(), Ordering::Relaxed);
        Some([
            f32::from_bits(bands[EQ_LOW].load(Ordering::Relaxed)),
            f32::from_bits(bands[EQ_MID].load(Ordering::Relaxed)),
            f32::from_bits(bands[EQ_HIGH].load(Ordering::Relaxed)),
        ])
    }
}

/// Finite EQ gain clamped to the engine's ±26 dB range.
pub(crate) fn clamp_eq_db(value: f32) -> Option<f32> {
    value
        .is_finite()
        .then_some(value.clamp(-(EQ_LIMIT_DB as f32), EQ_LIMIT_DB as f32))
}

/// A controller action the native path owns. `mirror` asks the UI to reflect the new value
/// without re-sending it to the engine.
pub(crate) struct NativeAction {
    pub message: Option<EngineMsg>,
    pub mirror: bool,
}

pub(crate) struct FilterSettings {
    pub mode: FilterMode,
    pub cutoff: f32,
    pub resonance: f32,
}

/// Bipolar DJ filter knob: −1 = full low-pass, 0 = off, +1 = full high-pass.
pub(crate) fn filter_settings(position: f64) -> FilterSettings {
    if position.abs() < FILTER_DEAD_ZONE {
        return FilterSettings {
            mode: FilterMode::Off,
            cutoff: 1000.0,
            resonance: 0.9,
        };
    }
    if position < 0.0 {
        return FilterSettings {
            mode: FilterMode::LowPass,
            cutoff: (20_000.0 * (200.0_f64 / 20_000.0).powf(-position)) as f32,
            resonance: (0.9 - position) as f32,
        };
    }
    FilterSettings {
        mode: FilterMode::HighPass,
        cutoff: (20.0 * (4000.0_f64 / 20.0).powf(position)) as f32,
        resonance: (0.9 + position) as f32,
    }
}

fn deck_control(control: &str) -> Option<(usize, &str)> {
    let rest = control.strip_prefix("deck.")?;
    let (index, name) = rest.split_once('.')?;
    let deck = index.parse::<usize>().ok()?;
    (deck < NUM_DECKS).then_some((deck, name))
}

/// Resolve a controller update to an engine command when the control is natively owned.
/// `None` means the control still belongs to the UI command path.
pub(crate) fn resolve(
    control: &str,
    value: f64,
    telemetry: &DeckTelemetry,
    eq: &DeckEqState,
) -> Option<NativeAction> {
    let (deck, name) = deck_control(control)?;
    if !value.is_finite() {
        return Some(NativeAction {
            message: None,
            mirror: false,
        });
    }
    let pressed = value >= 0.5;
    let mirrored = |message: EngineMsg| {
        Some(NativeAction {
            message: Some(message),
            mirror: true,
        })
    };
    let unmirrored = |message: Option<EngineMsg>| {
        Some(NativeAction {
            message,
            mirror: false,
        })
    };
    let eq_band = |band: usize| {
        let db = value.clamp(-EQ_LIMIT_DB, EQ_LIMIT_DB) as f32;
        eq.with_band(deck, band, db)?;
        mirrored(EngineMsg::DeckEqBand { deck, band, db })
    };
    match name {
        "volume" => mirrored(EngineMsg::DeckGain {
            deck,
            gain: value.clamp(0.0, 1.0) as f32,
        }),
        "trim" => mirrored(EngineMsg::DeckTrim {
            deck,
            gain: value.clamp(0.0, TRIM_MAX) as f32,
        }),
        "eq.low" => eq_band(EQ_LOW),
        "eq.mid" => eq_band(EQ_MID),
        "eq.high" => eq_band(EQ_HIGH),
        "filter" => {
            let settings = filter_settings(value.clamp(-1.0, 1.0));
            mirrored(EngineMsg::DeckFilter {
                deck,
                mode: settings.mode,
                cutoff: settings.cutoff,
                resonance: settings.resonance,
            })
        }
        "tempo" => mirrored(EngineMsg::DeckTempo {
            deck,
            ratio: 1.0 + value.clamp(-TEMPO_LIMIT_PERCENT, TEMPO_LIMIT_PERCENT) / 100.0,
        }),
        "play" => {
            unmirrored(
                (pressed && telemetry.is_loaded(deck)).then(|| EngineMsg::DeckPlaying {
                    deck,
                    playing: !telemetry.is_playing(deck),
                }),
            )
        }
        "cue" => unmirrored(Some(EngineMsg::CueButton { deck, pressed })),
        "censor" => unmirrored(Some(EngineMsg::SetCensor {
            deck,
            active: pressed,
        })),
        "start" => unmirrored(pressed.then_some(EngineMsg::DeckSeek { deck, frame: 0.0 })),
        _ => None,
    }
}

/// Playhead positions captured when a controller message arrives, so cue/loop/jump actions use
/// the moment of the press instead of the last 30 Hz UI telemetry sample.
pub(crate) struct CapturedPosition {
    /// Engine read position.
    pub frame: f64,
    /// Position currently leaving the speakers (read position minus output latency).
    pub heard_frame: f64,
}

pub(crate) fn capture_position(
    control: &str,
    telemetry: &DeckTelemetry,
) -> Option<CapturedPosition> {
    let (deck, _) = deck_control(control)?;
    if !telemetry.is_loaded(deck) {
        return None;
    }
    let frame = telemetry.playhead_frames(deck);
    let latency_frames = telemetry.deck_rate(deck) * f64::from(telemetry.output_latency_secs());
    let heard_frame = (frame - latency_frames).max(0.0);
    (frame.is_finite() && heard_frame.is_finite())
        .then_some(CapturedPosition { frame, heard_frame })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(control: &str, value: f64, eq: &DeckEqState) -> NativeAction {
        resolve(control, value, &DeckTelemetry::new(), eq).expect("native control")
    }

    #[test]
    fn ui_owned_and_out_of_range_deck_controls_are_not_native() {
        let eq = DeckEqState::default();
        let telemetry = DeckTelemetry::new();
        for control in [
            "deck.0.hotcue.0",
            "deck.0.sync",
            "deck.0.gain",
            "mixer.crossfader",
            "deck.4.volume",
            "deck.x.volume",
        ] {
            assert!(
                resolve(control, 1.0, &telemetry, &eq).is_none(),
                "{control}"
            );
        }
    }

    #[test]
    fn eq_db_rejects_non_finite_and_clamps_to_the_engine_range() {
        assert_eq!(clamp_eq_db(40.0), Some(26.0));
        assert_eq!(clamp_eq_db(-40.0), Some(-26.0));
        assert_eq!(clamp_eq_db(3.5), Some(3.5));
        assert!(clamp_eq_db(f32::NAN).is_none());
        assert!(clamp_eq_db(f32::INFINITY).is_none());
    }

    #[test]
    fn eq_band_updates_preserve_the_other_bands() {
        let eq = DeckEqState::default();
        eq.record(1, -3.0, 2.0, 4.0);
        let action = resolved("deck.1.eq.mid", -40.0, &eq);
        assert!(action.mirror);
        assert!(matches!(
            action.message,
            Some(EngineMsg::DeckEqBand { deck: 1, band: 1, db }) if db == -26.0
        ));
        assert_eq!(eq.with_band(1, 1, -26.0), Some([-3.0, -26.0, 4.0]));
    }

    #[test]
    fn continuous_controls_clamp_to_engine_ranges() {
        let eq = DeckEqState::default();
        assert!(matches!(
            resolved("deck.0.volume", 2.0, &eq).message,
            Some(EngineMsg::DeckGain { deck: 0, gain }) if gain == 1.0
        ));
        assert!(matches!(
            resolved("deck.0.trim", -1.0, &eq).message,
            Some(EngineMsg::DeckTrim { deck: 0, gain }) if gain == 0.0
        ));
        assert!(matches!(
            resolved("deck.2.tempo", 8.0, &eq).message,
            Some(EngineMsg::DeckTempo { deck: 2, ratio }) if (ratio - 1.08).abs() < 1e-12
        ));
    }

    #[test]
    fn invalid_values_are_consumed_without_engine_commands() {
        let eq = DeckEqState::default();
        let action = resolved("deck.0.volume", f64::NAN, &eq);
        assert!(action.message.is_none());
        assert!(!action.mirror);
    }

    #[test]
    fn transport_follows_press_edges_and_requires_a_loaded_deck() {
        let eq = DeckEqState::default();
        assert!(resolved("deck.0.play", 1.0, &eq).message.is_none());
        assert!(resolved("deck.0.play", 0.0, &eq).message.is_none());
        assert!(resolved("deck.0.start", 0.0, &eq).message.is_none());
        assert!(matches!(
            resolved("deck.0.start", 1.0, &eq).message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 0.0
        ));
        for (value, expected) in [(1.0, true), (0.0, false)] {
            let action = resolved("deck.1.cue", value, &eq);
            assert!(!action.mirror);
            assert!(matches!(
                action.message,
                Some(EngineMsg::CueButton { deck: 1, pressed }) if pressed == expected
            ));
        }
    }

    #[test]
    fn filter_matches_the_ui_curve() {
        let off = filter_settings(0.01);
        assert!(matches!(off.mode, FilterMode::Off));
        let low_pass = filter_settings(-1.0);
        assert!(matches!(low_pass.mode, FilterMode::LowPass));
        assert!((low_pass.cutoff - 200.0).abs() < 0.01);
        assert!((low_pass.resonance - 1.9).abs() < 1e-6);
        let high_pass = filter_settings(1.0);
        assert!(matches!(high_pass.mode, FilterMode::HighPass));
        assert!((high_pass.cutoff - 4000.0).abs() < 0.01);
    }

    #[test]
    fn position_capture_requires_a_loaded_deck() {
        assert!(capture_position("deck.0.hotcue.0", &DeckTelemetry::new()).is_none());
        assert!(capture_position("mixer.crossfader", &DeckTelemetry::new()).is_none());
    }
}
