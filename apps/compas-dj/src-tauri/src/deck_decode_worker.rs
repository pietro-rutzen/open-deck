//! One decoder, at most one pending request per deck. Never used by the audio callback.
use compas_audio::NUM_DECKS;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send + 'static>;
struct Pending {
    slots: [Option<Job>; NUM_DECKS],
    next: usize,
}
impl Pending {
    fn take(&mut self) -> Option<Job> {
        for offset in 0..NUM_DECKS {
            let deck = (self.next + offset) % NUM_DECKS;
            if let Some(job) = self.slots[deck].take() {
                self.next = (deck + 1) % NUM_DECKS;
                return Some(job);
            }
        }
        None
    }
}
pub(crate) struct DecodeWorker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
}
impl DecodeWorker {
    fn new() -> Result<Self, String> {
        let pending = Arc::new((
            Mutex::new(Pending {
                slots: std::array::from_fn(|_| None),
                next: 0,
            }),
            Condvar::new(),
        ));
        let worker = pending.clone();
        std::thread::Builder::new()
            .name("deck-decoder".into())
            .spawn(move || loop {
                let job = {
                    let (lock, wake) = &*worker;
                    let mut queue = lock.lock().expect("decoder queue poisoned");
                    loop {
                        if let Some(job) = queue.take() {
                            break job;
                        }
                        queue = wake.wait(queue).expect("decoder queue poisoned");
                    }
                };
                // A failed job must never take down the shared decoder. The application job
                // catches its own panic to publish a generation-checked error as well.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            })
            .map_err(|e| format!("Cannot start track decoder: {e}"))?;
        Ok(Self { pending })
    }
    pub(crate) fn shared() -> Result<&'static Self, String> {
        static WORKER: OnceLock<Result<DecodeWorker, String>> = OnceLock::new();
        WORKER.get_or_init(Self::new).as_ref().map_err(Clone::clone)
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
        self.pending.1.notify_one();
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
        let ticket = loads.begin(1, || {}).unwrap();
        let job_loads = loads.clone();
        let (result, done) = mpsc::channel();
        loads
            .commit(1, ticket, || {
                worker.submit(
                    1,
                    Box::new(move || {
                        result.send(job_loads.current(1, ticket).unwrap()).unwrap();
                    }),
                )
            })
            .unwrap();
        loads.begin(1, || {}).unwrap(); // Eject invalidates the waiting request.
        release.send(()).unwrap();
        assert!(!done.recv_timeout(Duration::from_secs(5)).unwrap());
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
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 10_000);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 9_999);
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
}
