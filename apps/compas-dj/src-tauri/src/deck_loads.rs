//! Per-deck asynchronous load admission. Never hold a gate while decoding/analyzing.
//! A gate covers only latest-request validation, command enqueue and publication; it is not an
//! acknowledgement that the audio callback has installed a track.
use compas_audio::NUM_DECKS;
use std::sync::Mutex;

pub(crate) struct DeckLoads([Mutex<u64>; NUM_DECKS]);
impl Default for DeckLoads {
    fn default() -> Self {
        Self(std::array::from_fn(|_| Mutex::new(0)))
    }
}
impl DeckLoads {
    /// Invalidate all previous workers for this deck, publishing loading under the same gate.
    pub(crate) fn begin(&self, deck: usize, publish: impl FnOnce(u64)) -> Result<u64, String> {
        let mut generation = self
            .0
            .get(deck)
            .ok_or("Invalid load deck")?
            .lock()
            .map_err(|e| e.to_string())?;
        let ticket = generation
            .checked_add(1)
            .ok_or("Deck load generation exhausted")?;
        *generation = ticket;
        publish(ticket);
        Ok(ticket)
    }
    pub(crate) fn current(&self, deck: usize, ticket: u64) -> Result<bool, String> {
        Ok(*self
            .0
            .get(deck)
            .ok_or("Invalid load deck")?
            .lock()
            .map_err(|e| e.to_string())?
            == ticket)
    }
    /// Drop every in-flight load. Used when the output device is replaced and decks are cleared.
    pub(crate) fn invalidate_all(&self) -> Result<(), String> {
        for deck in 0..NUM_DECKS {
            self.begin(deck, |_| {})?;
        }
        Ok(())
    }
    /// Retire stale completions without publishing errors, installing native state or enqueueing.
    pub(crate) fn commit<T>(
        &self,
        deck: usize,
        ticket: u64,
        apply: impl FnOnce() -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let generation = self
            .0
            .get(deck)
            .ok_or("Invalid load deck")?
            .lock()
            .map_err(|e| e.to_string())?;
        if *generation != ticket {
            return Ok(None);
        }
        apply().map(Some)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_completions_cannot_replace_latest_track() {
        let loads = DeckLoads::default();
        let old = loads.begin(0, |_| {}).unwrap();
        let new = loads.begin(0, |_| {}).unwrap();
        let mut installed = Vec::new();
        assert!(loads
            .commit(0, new, || {
                installed.push("new");
                Ok(())
            })
            .unwrap()
            .is_some());
        assert!(loads
            .commit(0, old, || {
                installed.push("old");
                Ok(())
            })
            .unwrap()
            .is_none());
        assert_eq!(installed, vec!["new"]);
    }
    #[test]
    fn unload_invalidates_decode_and_other_deck_is_independent() {
        let loads = DeckLoads::default();
        let a = loads.begin(0, |_| {}).unwrap();
        let b = loads.begin(1, |_| {}).unwrap();
        let unload = loads.begin(0, |_| {}).unwrap();
        loads.commit(0, unload, || Ok(())).unwrap();
        assert!(!loads.current(0, a).unwrap());
        assert!(loads.current(1, b).unwrap());
        assert!(loads
            .commit::<()>(0, a, || panic!("stale error/install published"))
            .unwrap()
            .is_none());
        assert!(loads
            .begin(NUM_DECKS, |_| panic!("invalid admission"))
            .is_err());
    }
    #[test]
    fn admission_waits_until_commit_publication_finishes() {
        use std::sync::{mpsc, Arc};
        let loads = Arc::new(DeckLoads::default());
        let ticket = loads.begin(0, |_| {}).unwrap();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = loads.clone();
        let commit = std::thread::spawn(move || {
            worker
                .commit(0, ticket, || {
                    inside_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap()
        });
        inside_rx.recv().unwrap();
        // The admission closure cannot publish while a current completion owns the gate.
        let (admit_tx, admit_rx) = mpsc::channel();
        let worker = loads.clone();
        let admission =
            std::thread::spawn(move || worker.begin(0, |_| admit_tx.send(()).unwrap()).unwrap());
        assert!(admit_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        commit.join().unwrap();
        let next = admission.join().unwrap();
        admit_rx.recv().unwrap();
        assert!(!loads.current(0, ticket).unwrap());
        assert!(loads.current(0, next).unwrap());
    }
}
