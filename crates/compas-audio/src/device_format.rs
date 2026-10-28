//! Pick a stream format that keeps the sample rate a device is already running.

use cpal::{SampleFormat, SampleRate, SupportedStreamConfig, SupportedStreamConfigRange};

/// `active_rate_hz` is the rate the device is already using, when the host can report it.
/// The chosen format keeps that rate when a supported range contains it. Channel count and
/// sample format stay with the device's current format when that combination supports the rate.
/// An unknown or unsupported rate leaves `current` unchanged.
pub(crate) fn config_at_active_rate(
    current: SupportedStreamConfig,
    supported: impl IntoIterator<Item = SupportedStreamConfigRange>,
    active_rate_hz: Option<u32>,
) -> SupportedStreamConfig {
    let Some(active_rate_hz) = active_rate_hz.filter(|rate| *rate > 0) else {
        return current;
    };
    if active_rate_hz == current.sample_rate().0 {
        return current;
    }
    let channels = current.channels();
    let format = current.sample_format();
    let ranges: Vec<SupportedStreamConfigRange> = supported.into_iter().collect();
    ranges
        .iter()
        .filter(|range| range_contains(range, active_rate_hz))
        .min_by_key(|range| format_rank(range, channels, format))
        .map(|range| range.with_sample_rate(SampleRate(active_rate_hz)))
        .unwrap_or(current)
}

fn range_contains(range: &SupportedStreamConfigRange, rate: u32) -> bool {
    (range.min_sample_rate().0..=range.max_sample_rate().0).contains(&rate)
}

fn format_rank(range: &SupportedStreamConfigRange, channels: u16, format: SampleFormat) -> u8 {
    match (
        range.channels() == channels,
        range.sample_format() == format,
        range.sample_format() == SampleFormat::F32,
    ) {
        (true, true, _) => 0,
        (true, false, true) => 1,
        (true, false, false) => 2,
        (false, true, _) => 3,
        (false, false, true) => 4,
        (false, false, false) => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::config_at_active_rate;
    use cpal::{SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfigRange};

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            SampleRate(min),
            SampleRate(max),
            SupportedBufferSize::Unknown,
            format,
        )
    }

    #[test]
    fn unknown_active_rate_keeps_the_current_format() {
        let formats = [range(2, 44_100, 48_000, SampleFormat::F32)];
        let current = formats[0].with_sample_rate(SampleRate(44_100));
        let chosen = config_at_active_rate(current, formats, None);
        assert_eq!(chosen.sample_rate().0, 44_100);
        assert_eq!(chosen.channels(), 2);
    }

    #[test]
    fn active_rate_replaces_a_different_default() {
        let formats = [range(4, 44_100, 96_000, SampleFormat::F32)];
        let current = formats[0].with_sample_rate(SampleRate(44_100));
        let chosen = config_at_active_rate(current, formats, Some(48_000));
        assert_eq!(chosen.sample_rate().0, 48_000);
        assert_eq!(chosen.channels(), 4);
        assert_eq!(chosen.sample_format(), SampleFormat::F32);
    }

    #[test]
    fn matching_rate_keeps_channel_count_and_format() {
        let formats = [
            range(2, 48_000, 48_000, SampleFormat::I16),
            range(4, 44_100, 48_000, SampleFormat::F32),
        ];
        let current = formats[1].with_sample_rate(SampleRate(44_100));
        let chosen = config_at_active_rate(current, formats, Some(48_000));
        assert_eq!(chosen.sample_rate().0, 48_000);
        assert_eq!(chosen.channels(), 4);
        assert_eq!(chosen.sample_format(), SampleFormat::F32);
    }

    #[test]
    fn unsupported_active_rate_keeps_the_current_format() {
        let formats = [range(2, 44_100, 48_000, SampleFormat::F32)];
        let current = formats[0].with_sample_rate(SampleRate(44_100));
        let chosen = config_at_active_rate(current, formats, Some(192_000));
        assert_eq!(chosen.sample_rate().0, 44_100);
    }

    #[test]
    fn active_rate_can_change_format_when_that_is_the_only_match() {
        let formats = [
            range(2, 44_100, 44_100, SampleFormat::F32),
            range(2, 48_000, 48_000, SampleFormat::I16),
        ];
        let current = formats[0].with_sample_rate(SampleRate(44_100));
        let chosen = config_at_active_rate(current, formats, Some(48_000));
        assert_eq!(chosen.sample_rate().0, 48_000);
        assert_eq!(chosen.sample_format(), SampleFormat::I16);
    }
}
