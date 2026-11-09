//! Remembers the deck buffers the audio thread has actually sent, so a failed
//! output stream can be rebuilt without asking the UI to reload.
//! A speaker change the user asked for does not use this: that path clears the decks.

use std::collections::VecDeque;
use std::sync::Arc;

use compas_audio::{AudioCommand, AudioEngine, DeckBuffer, DeckTelemetry, NUM_DECKS};

struct DeckMemory {
    buffer: Arc<DeckBuffer>,
    beat_offset: f64,
    beat_interval: f64,
}

pub(crate) struct DeckRestore {
    decks: [Option<DeckMemory>; NUM_DECKS],
}

pub(crate) enum Remembered {
    Load {
        deck: usize,
        buffer: Arc<DeckBuffer>,
        beat_offset: f64,
        beat_interval: f64,
    },
    Unload {
        deck: usize,
    },
    Beatgrid {
        deck: usize,
        offset: f64,
        interval: f64,
    },
    Skip,
}

struct Heard {
    loaded: bool,
    playing: bool,
    playhead: f64,
    tempo: f64,
}

impl Default for DeckRestore {
    fn default() -> Self {
        Self {
            decks: std::array::from_fn(|_| None),
        }
    }
}

impl DeckRestore {
    pub(crate) fn note(command: &AudioCommand) -> Remembered {
        match command {
            AudioCommand::LoadDeck {
                deck,
                buffer,
                beat_offset,
                beat_interval,
            } => Remembered::Load {
                deck: *deck,
                buffer: Arc::clone(buffer),
                beat_offset: *beat_offset,
                beat_interval: *beat_interval,
            },
            AudioCommand::UnloadDeck { deck } => Remembered::Unload { deck: *deck },
            AudioCommand::SetBeatgrid {
                deck,
                offset,
                interval,
            } => Remembered::Beatgrid {
                deck: *deck,
                offset: *offset,
                interval: *interval,
            },
            _ => Remembered::Skip,
        }
    }

    pub(crate) fn apply(&mut self, remembered: Remembered) {
        match remembered {
            Remembered::Load {
                deck,
                buffer,
                beat_offset,
                beat_interval,
            } => {
                if let Some(slot) = self.decks.get_mut(deck) {
                    *slot = Some(DeckMemory {
                        buffer,
                        beat_offset,
                        beat_interval,
                    });
                }
            }
            Remembered::Unload { deck } => {
                if let Some(slot) = self.decks.get_mut(deck) {
                    *slot = None;
                }
            }
            Remembered::Beatgrid {
                deck,
                offset,
                interval,
            } => {
                if let Some(Some(memory)) = self.decks.get_mut(deck) {
                    memory.beat_offset = offset;
                    memory.beat_interval = interval;
                }
            }
            Remembered::Skip => {}
        }
    }

    pub(crate) fn replay(&self, telemetry: &DeckTelemetry) -> Vec<AudioCommand> {
        let heard = std::array::from_fn(|deck| Heard {
            loaded: telemetry.is_loaded(deck),
            playing: telemetry.is_playing(deck),
            playhead: telemetry.playhead_frames(deck),
            tempo: telemetry.tempo_ratio(deck),
        });
        self.commands_for(&heard)
    }

    fn commands_for(&self, heard: &[Heard; NUM_DECKS]) -> Vec<AudioCommand> {
        let mut commands = Vec::new();
        for (deck, (memory, heard)) in self.decks.iter().zip(heard.iter()).enumerate() {
            let Some(memory) = memory else {
                continue;
            };
            if !heard.loaded {
                continue;
            }
            let tempo = if heard.tempo.is_finite() && heard.tempo >= 0.05 {
                heard.tempo
            } else {
                1.0
            };
            commands.push(AudioCommand::LoadDeck {
                deck,
                buffer: Arc::clone(&memory.buffer),
                beat_offset: memory.beat_offset,
                beat_interval: memory.beat_interval,
            });
            commands.push(AudioCommand::SeekDeck {
                deck,
                frame: heard.playhead,
            });
            commands.push(AudioCommand::SetDeckTempo { deck, ratio: tempo });
            if heard.playing {
                commands.push(AudioCommand::SetDeckPlaying {
                    deck,
                    playing: true,
                });
            }
        }
        commands
    }
}

pub(crate) fn flush(engine: &mut AudioEngine, queued: &mut VecDeque<AudioCommand>) -> bool {
    while let Some(command) = queued.pop_front() {
        if let Err(command) = engine.try_send_command(command) {
            queued.push_front(command);
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(value: f32) -> Arc<DeckBuffer> {
        Arc::new(DeckBuffer::new(vec![value; 64], 48_000))
    }

    fn heard(loaded: bool, playing: bool, playhead: f64, tempo: f64) -> Heard {
        Heard {
            loaded,
            playing,
            playhead,
            tempo,
        }
    }

    #[test]
    fn a_stream_restart_reloads_each_heard_deck_at_its_own_playhead() {
        let mut restore = DeckRestore::default();
        let first = buffer(0.25);
        let second = buffer(-0.25);
        restore.apply(Remembered::Load {
            deck: 0,
            buffer: Arc::clone(&first),
            beat_offset: 4.0,
            beat_interval: 8.0,
        });
        restore.apply(Remembered::Load {
            deck: 1,
            buffer: Arc::clone(&second),
            beat_offset: 1.0,
            beat_interval: 2.0,
        });
        restore.apply(Remembered::Beatgrid {
            deck: 0,
            offset: 9.0,
            interval: 10.0,
        });
        let mut heard_decks = std::array::from_fn(|_| heard(false, false, 0.0, 1.0));
        heard_decks[0] = heard(true, true, 1_500.0, 1.25);
        heard_decks[1] = heard(true, false, 40.0, 0.0);
        let commands = restore.commands_for(&heard_decks);
        assert!(matches!(
            &commands[0],
            AudioCommand::LoadDeck { deck: 0, beat_offset, beat_interval, .. }
                if *beat_offset == 9.0 && *beat_interval == 10.0
        ));
        assert!(matches!(
            &commands[1],
            AudioCommand::SeekDeck { deck: 0, frame } if *frame == 1_500.0
        ));
        assert!(matches!(
            &commands[2],
            AudioCommand::SetDeckTempo { deck: 0, ratio } if *ratio == 1.25
        ));
        assert!(matches!(
            &commands[3],
            AudioCommand::SetDeckPlaying {
                deck: 0,
                playing: true
            }
        ));
        assert!(matches!(
            &commands[4],
            AudioCommand::LoadDeck { deck: 1, .. }
        ));
        assert!(matches!(
            &commands[5],
            AudioCommand::SeekDeck { deck: 1, frame } if *frame == 40.0
        ));
        assert!(matches!(
            &commands[6],
            AudioCommand::SetDeckTempo { deck: 1, ratio } if *ratio == 1.0
        ));
        assert_eq!(commands.len(), 7);
    }

    #[test]
    fn a_load_the_engine_never_heard_is_not_restored() {
        let mut restore = DeckRestore::default();
        restore.apply(Remembered::Load {
            deck: 0,
            buffer: buffer(0.5),
            beat_offset: 0.0,
            beat_interval: 0.0,
        });
        restore.apply(Remembered::Unload { deck: 0 });
        restore.apply(Remembered::Load {
            deck: 1,
            buffer: buffer(0.5),
            beat_offset: 0.0,
            beat_interval: 0.0,
        });
        let heard_decks = std::array::from_fn(|_| heard(false, false, 12.0, 1.0));
        assert!(restore.commands_for(&heard_decks).is_empty());
    }
}
