//! Offline command/routing regression, not a device or callback-deadline benchmark.
use super::*;

fn run_session(seconds: usize) {
    let (mut tx, rx) = rtrb::RingBuffer::new(32);
    let (retire_tx, mut retired) = rtrb::RingBuffer::new(8);
    let mut mixer = Mixer::new(48_000.0, rx, retire_tx, Arc::new(DeckTelemetry::new()));
    let fixtures = [0.2, 0.4].map(|v| Arc::new(DeckBuffer::new(vec![v; 96_000], 48_000)));
    for cycle in 0..seconds {
        // Replace both decks every second, including an already-playing deck.
        for (deck, fixture) in fixtures.iter().enumerate() {
            tx.push(AudioCommand::LoadDeck {
                deck,
                buffer: fixture.clone(),
                beat_offset: 0.0,
                beat_interval: 24_000.0,
            })
            .unwrap();
            tx.push(AudioCommand::SetLoop {
                deck,
                in_frame: 0.0,
                out_frame: 12_000.0,
                active: true,
            })
            .unwrap();
            tx.push(AudioCommand::SetDeckPlaying {
                deck,
                playing: true,
            })
            .unwrap();
            tx.push(AudioCommand::SetDeckCue {
                deck,
                active: deck == cycle % 2,
            })
            .unwrap();
            tx.push(AudioCommand::SetDeckGain { deck, gain: 1.0 })
                .unwrap();
            tx.push(AudioCommand::SetDeckXfaderAssign {
                deck,
                assign: if deck == 0 { 0 } else { 2 },
            })
            .unwrap();
        }
        let selected = cycle % 2;
        tx.push(AudioCommand::SetCrossfader(selected as f32))
            .unwrap();
        tx.push(AudioCommand::SetCueMix(0.0)).unwrap();
        tx.push(AudioCommand::SetCueVolume(1.0)).unwrap();
        mixer.drain_commands();
        while retired.pop().is_ok() {}
        for phase in 0..3 {
            if phase == 1 {
                tx.push(AudioCommand::SetDeckGain {
                    deck: selected,
                    gain: 0.0,
                })
                .unwrap();
            } else if phase == 2 {
                tx.push(AudioCommand::SetDeckPlaying {
                    deck: selected,
                    playing: false,
                })
                .unwrap();
            }
            mixer.drain_commands();
            let mut master = 0.0f64;
            let mut cue = 0.0f64;
            for frame in 0..16_000 {
                let buses = mixer.next_buses();
                for sample in [buses.master.0, buses.master.1, buses.cue.0, buses.cue.1] {
                    assert!(
                        sample.is_finite() && sample.abs() <= 1.0,
                        "cycle {cycle}, phase {phase}: {sample}"
                    );
                }
                // Exclude gain-ramp settling; assert sustained output, not a single sample.
                if frame >= 8_000 {
                    master += buses.master.0.abs() as f64;
                    cue += buses.cue.0.abs() as f64;
                }
            }
            match phase {
                0 => assert!(
                    master > 500.0 && cue > 500.0,
                    "audible deck: {master}, {cue}"
                ),
                1 => assert!(
                    master < 0.1 && cue > 500.0,
                    "PFL independent of fader: {master}, {cue}"
                ),
                _ => assert!(master < 0.1 && cue < 0.1, "paused deck: {master}, {cue}"),
            }
            assert!(mixer.decks[selected].playhead < 12_000.0, "loop escaped");
        }
        for fixture in &fixtures {
            assert_eq!(
                Arc::strong_count(fixture),
                2,
                "only fixture and loaded deck retain audio"
            );
        }
    }
    for deck in 0..2 {
        tx.push(AudioCommand::UnloadDeck { deck }).unwrap();
    }
    mixer.drain_commands();
    while retired.pop().is_ok() {}
    for fixture in &fixtures {
        assert_eq!(Arc::strong_count(fixture), 1, "unload releases audio");
    }
    assert!(mixer.decks[..2].iter().all(|d| d.buffer.is_none()));
}

#[test]
fn repeated_load_cue_transport_session() {
    run_session(60);
}

#[test]
#[ignore = "Explicit offline hour; no hardware or wall-clock endurance claim"]
fn offline_hour_load_cue_transport_session() {
    run_session(3600);
}
