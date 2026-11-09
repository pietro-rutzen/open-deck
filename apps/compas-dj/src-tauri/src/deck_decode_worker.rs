//! One decoder thread per deck, and at most one pending request on that deck.
//! A long decode cannot hold the other deck's load. Never used by the audio callback.
use compas_audio::NUM_DECKS;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send + 'static>;
struct Pending {
    slots: [Option<Job>; NUM_DECKS],
}
pub(crate) struct DecodeWorker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
}
impl DecodeWorker {
    fn new() -> Result<Self, String> {
        Self::spawn("deck-decoder")
    }
    fn spawn(name: &'static str) -> Result<Self, String> {
        let pending = Arc::new((
            Mutex::new(Pending {
                slots: std::array::from_fn(|_| None),
            }),
            Condvar::new(),
        ));
        for deck in 0..NUM_DECKS {
            let worker = pending.clone();
            std::thread::Builder::new()
                .name(format!("{name}-{deck}"))
                .spawn(move || loop {
                    let job = {
                        let (lock, wake) = &*worker;
                        let mut queue = lock.lock().expect("decoder queue poisoned");
                        loop {
                            if let Some(job) = queue.slots[deck].take() {
                                break job;
                            }
                            queue = wake.wait(queue).expect("decoder queue poisoned");
                        }
                    };
                    // A failed job must never take down this deck's decoder. The application
                    // job catches its own panic to publish a generation-checked error as well.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                })
                .map_err(|e| format!("Cannot start track decoder: {e}"))?;
        }
        Ok(Self { pending })
    }
    pub(crate) fn shared() -> Result<&'static Self, String> {
        static WORKER: OnceLock<Result<DecodeWorker, String>> = OnceLock::new();
        WORKER.get_or_init(Self::new).as_ref().map_err(Clone::clone)
    }
    /// One analysis job per deck. A newer load replaces a job that has not started.
    pub(crate) fn analysis() -> Result<&'static Self, String> {
        static WORKER: OnceLock<Result<DecodeWorker, String>> = OnceLock::new();
        WORKER
            .get_or_init(|| Self::spawn("deck-analysis"))
            .as_ref()
            .map_err(Clone::clone)
    }
    /// Call under DeckLoads::commit so an older submission cannot replace a newer slot.
    pub(crate) fn submit(&self, deck: usize, job: Job) -> Result<(), String> {
        if deck >= NUM_DECKS {
            return Err("Invalid load deck".into());
        }
        let old = {
            let mut queue = self.pending.0.lock().map_err(|e| e.to_string())?;
            queue.slots[deck].replace(job)
        };
        drop(old);
        self.pending.1.notify_all();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    #[test]
    fn queued_ejected_track_never_starts_decode() {
        let worker = DecodeWorker::new().unwrap();
        let loads = Arc::new(crate::deck_loads::DeckLoads::default());
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        worker
            .submit(
                0,
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let ticket = loads.begin(0, |_| {}).unwrap();
        let job_loads = loads.clone();
        let (result, done) = mpsc::channel();
        loads
            .commit(0, ticket, || {
                worker.submit(
                    0,
                    Box::new(move || {
                        result.send(job_loads.current(0, ticket).unwrap()).unwrap();
                    }),
                )
            })
            .unwrap();
        loads.begin(0, |_| {}).unwrap(); // Eject invalidates the waiting request.
        release.send(()).unwrap();
        assert!(!done.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn a_long_decode_does_not_hold_the_other_deck() {
        let worker = DecodeWorker::new().unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        worker
            .submit(
                0,
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        worker
            .submit(1, Box::new(move || done_tx.send(()).unwrap()))
            .unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
    }

    #[test]
    fn burst_is_coalesced_other_deck_progresses_and_panics_do_not_kill_worker() {
        let worker = DecodeWorker::new().unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        worker
            .submit(
                0,
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (tx, rx) = mpsc::channel();
        for i in 0..10_000 {
            let tx = tx.clone();
            worker
                .submit(
                    0,
                    Box::new(move || {
                        tx.send(i).unwrap();
                    }),
                )
                .unwrap();
        }
        let other = tx.clone();
        worker
            .submit(
                1,
                Box::new(move || {
                    other.send(10_000).unwrap();
                }),
            )
            .unwrap();
        release.send(()).unwrap();
        let mut finished = [
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        ];
        finished.sort_unstable();
        assert_eq!(finished, [9_999, 10_000]);
        assert!(rx.try_recv().is_err());
        let (entered, started) = mpsc::channel();
        worker
            .submit(
                2,
                Box::new(move || {
                    entered.send(()).unwrap();
                    panic!("failed decode");
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        worker
            .submit(
                3,
                Box::new(move || {
                    tx.send(42).unwrap();
                }),
            )
            .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 42);
        assert!(worker.submit(NUM_DECKS, Box::new(|| {})).is_err());
    }

    fn tiny_wav(name: &str, rate: u32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-worker-{name}-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&1_000i16.to_le_bytes());
        wav.extend_from_slice(&(-1_000i16).to_le_bytes());
        std::fs::write(&path, wav).unwrap();
        path
    }

    #[test]
    fn a_bad_load_does_not_block_the_other_deck_or_keep_the_old_generation() {
        let worker = DecodeWorker::new().unwrap();
        let loads = Arc::new(crate::deck_loads::DeckLoads::default());
        let good = tiny_wav("good", 48_000);
        let replacement = tiny_wav("next", 22_050);
        let missing = std::env::temp_dir().join(format!(
            "opendeck-worker-missing-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        worker
            .submit(
                0,
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();

        let installed = Arc::new(Mutex::new(Vec::new()));
        let first = loads.begin(0, |_| {}).unwrap();
        let second = loads.begin(0, |_| {}).unwrap();
        let other = loads.begin(1, |_| {}).unwrap();
        let submit = |deck, ticket, path: std::path::PathBuf| {
            let job_loads = loads.clone();
            let installed = installed.clone();
            loads
                .commit(deck, ticket, || {
                    let current = job_loads.clone();
                    worker.submit(
                        deck,
                        Box::new(move || {
                            if !current.current(deck, ticket).unwrap_or(false) {
                                return;
                            }
                            if let Ok(Some((buffer, metadata))) =
                                crate::deck_loading::decode(&path, || {
                                    current.current(deck, ticket).unwrap_or(false)
                                })
                            {
                                if current.current(deck, ticket).unwrap_or(false) {
                                    installed.lock().unwrap().push((
                                        deck,
                                        metadata.provider_id,
                                        buffer.frames(),
                                    ));
                                }
                            }
                        }),
                    )
                })
                .unwrap()
        };
        submit(0, first, missing);
        submit(0, second, replacement.clone());
        submit(1, other, good.clone());
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while installed.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut got = installed.lock().unwrap().clone();
        got.sort();
        assert_eq!(
            got,
            vec![
                (0, replacement.display().to_string(), 1),
                (1, good.display().to_string(), 1),
            ]
        );
        let _ = std::fs::remove_file(good);
        let _ = std::fs::remove_file(replacement);
    }

    #[test]
    fn a_load_finishes_while_analysis_is_still_running() {
        let analysis = DecodeWorker::new().unwrap();
        let decoder = DecodeWorker::new().unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        analysis
            .submit(
                0,
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let path = tiny_wav("during-analysis", 48_000);
        let (done, finished) = mpsc::channel();
        let decode_path = path.clone();
        decoder
            .submit(
                1,
                Box::new(move || {
                    let decoded = crate::deck_loading::decode(&decode_path, || true);
                    done.send(decoded.unwrap().unwrap().0.frames()).unwrap();
                }),
            )
            .unwrap();
        assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        release.send(()).unwrap();
        let _ = std::fs::remove_file(path);
    }
}
