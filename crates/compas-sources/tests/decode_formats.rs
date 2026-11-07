use std::fs;
use std::path::PathBuf;

use compas_sources::{decode_full, AudioSource, LocalFileSource, PcmSource};

const SAMPLE_RATE: u32 = 44_100;
const PCM_FRAMES: usize = 8_820;

struct Fixture {
    name: &'static str,
    container: &'static str,
    frames: usize,
    duration_ms: u64,
    lossless_pcm: bool,
}

const FIXTURES: [Fixture; 5] = [
    Fixture {
        name: "tone.wav",
        container: "wav",
        frames: PCM_FRAMES,
        duration_ms: 200,
        lossless_pcm: true,
    },
    Fixture {
        name: "tone.aiff",
        container: "aiff",
        frames: PCM_FRAMES,
        duration_ms: 200,
        lossless_pcm: true,
    },
    Fixture {
        name: "tone.aif",
        container: "aiff",
        frames: PCM_FRAMES,
        duration_ms: 200,
        lossless_pcm: true,
    },
    Fixture {
        name: "tone.m4a",
        container: "m4a",
        frames: 11_264,
        duration_ms: 255,
        lossless_pcm: false,
    },
    Fixture {
        name: "tone.aac",
        container: "adts",
        frames: 11_264,
        duration_ms: 255,
        lossless_pcm: false,
    },
];

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn assert_container(name: &str, container: &str, bytes: &[u8]) {
    let recognized = match container {
        "wav" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "aiff" => bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"AIFF"),
        "m4a" => bytes.get(4..12) == Some(b"ftypM4A "),
        "adts" => bytes.get(..2) == Some(&[0xff, 0xf1]),
        _ => false,
    };
    assert!(recognized, "{name} is not a {container} file");
}

#[test]
fn generated_fixtures_decode_with_the_expected_container() {
    let mut lossless_pcm = None;
    for fixture in FIXTURES {
        let path = fixture_path(fixture.name);
        let bytes = fs::read(&path).expect(fixture.name);
        assert_container(fixture.name, fixture.container, &bytes);

        let source = LocalFileSource::open(&path)
            .unwrap_or_else(|error| panic!("{} failed to open: {error}", fixture.name));
        assert_eq!(source.sample_rate(), SAMPLE_RATE, "{}", fixture.name);
        assert_eq!(source.channels(), 2, "{}", fixture.name);
        let header_duration = source.metadata().duration_ms.expect(fixture.name);
        assert_eq!(header_duration, fixture.duration_ms, "{}", fixture.name);

        let (buffer, metadata) = decode_full(&path)
            .unwrap_or_else(|error| panic!("{} failed to decode: {error}", fixture.name));
        assert_eq!(buffer.source_rate, SAMPLE_RATE, "{}", fixture.name);
        assert_eq!(
            metadata.duration_ms,
            Some(header_duration),
            "{}",
            fixture.name
        );
        assert!(!buffer.samples.is_empty(), "{}", fixture.name);
        assert!(
            buffer.samples.iter().all(|sample| sample.is_finite()),
            "{}",
            fixture.name
        );
        let left_peak = buffer
            .samples
            .iter()
            .step_by(2)
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let right_peak = buffer
            .samples
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            left_peak > 0.02,
            "{name} left peak {left_peak}",
            name = fixture.name
        );
        assert!(
            right_peak > 0.02,
            "{name} right peak {right_peak}",
            name = fixture.name
        );
        assert_eq!(buffer.frames(), fixture.frames, "{}", fixture.name);
        assert_eq!(
            buffer.duration_ms(),
            fixture.duration_ms,
            "{}",
            fixture.name
        );
        if fixture.lossless_pcm {
            match &lossless_pcm {
                Some(reference) => assert_eq!(&buffer.samples, reference, "{}", fixture.name),
                None => lossless_pcm = Some(buffer.samples),
            }
        }
    }
}

#[test]
fn mpeg2_adts_from_afconvert_is_rejected() {
    let path = fixture_path("tone-mpeg2.aac");
    let bytes = fs::read(&path).expect("tone-mpeg2.aac");
    assert_eq!(&bytes[..2], &[0xff, 0xf9], "fixture is not MPEG-2 ADTS");
    let message = match LocalFileSource::open(&path) {
        Ok(_) => panic!("MPEG-2 ADTS should stay unsupported"),
        Err(error) => error.to_string(),
    };
    assert!(message.contains("unsupported format"), "{message}");
}
