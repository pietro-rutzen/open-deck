//! Control-thread command admission. Groups cannot interleave with other channel producers.
//! Queue remains unbounded until release reservation and all transactional callers are migrated.
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

const MAX_GROUP: usize = 16;
enum Envelope<T> {
    One(T),
    Group(VecDeque<T>),
}
#[derive(Default)]
struct Counts {
    closed: AtomicBool,
    outstanding: AtomicUsize,
    high_water: AtomicUsize,
}
pub(crate) struct Sender<T> {
    tx: mpsc::Sender<Envelope<T>>,
    counts: Arc<Counts>,
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            counts: self.counts.clone(),
        }
    }
}
pub(crate) struct Receiver<T> {
    rx: mpsc::Receiver<Envelope<T>>,
    pending: VecDeque<T>,
    counts: Arc<Counts>,
}
pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = mpsc::channel();
    let counts = Arc::new(Counts::default());
    (
        Sender {
            tx,
            counts: counts.clone(),
        },
        Receiver {
            rx,
            pending: VecDeque::new(),
            counts,
        },
    )
}
impl<T> Sender<T> {
    fn admit(&self, envelope: Envelope<T>, count: usize) -> Result<(), String> {
        // Reserve before send: receiver can run as soon as send publishes the envelope.
        let depth = self.counts.outstanding.fetch_add(count, Ordering::Relaxed) + count;
        self.counts.high_water.fetch_max(depth, Ordering::Relaxed);
        if self.tx.send(envelope).is_err() {
            self.counts.outstanding.fetch_sub(count, Ordering::Relaxed);
            return Err("Audio command receiver disconnected".into());
        }
        Ok(())
    }
    pub(crate) fn send(&self, value: T) -> Result<(), String> {
        self.admit(Envelope::One(value), 1)
    }
    pub(crate) fn send_group(&self, values: Vec<T>) -> Result<(), String> {
        if values.is_empty() || values.len() > MAX_GROUP {
            return Err("Invalid audio command group size".into());
        }
        let count = values.len();
        self.admit(Envelope::Group(values.into()), count)
    }
    /// Includes channel messages, an active group's tail, and concurrent send reservations.
    /// Dequeue is not callback application. High water can include a failed send reservation.
    pub(crate) fn diagnostics(&self) -> (usize, usize) {
        let outstanding = self.counts.outstanding.load(Ordering::Relaxed);
        (
            if self.counts.closed.load(Ordering::Acquire) {
                0
            } else {
                outstanding
            },
            self.counts.high_water.load(Ordering::Relaxed),
        )
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Keep send reservations intact so racing failures can roll themselves back.
        self.counts.closed.store(true, Ordering::Release);
    }
}
impl<T> Receiver<T> {
    pub(crate) fn recv_timeout(&mut self, timeout: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        let value = if let Some(value) = self.pending.pop_front() {
            value
        } else {
            match self.rx.recv_timeout(timeout)? {
                Envelope::One(value) => value,
                Envelope::Group(mut group) => {
                    let first = group.pop_front().expect("admitted groups are nonempty");
                    self.pending = group;
                    first
                }
            }
        };
        self.counts.outstanding.fetch_sub(1, Ordering::Relaxed);
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_hides_abandoned_group_tail_and_keeps_peak() {
        let (tx, mut rx) = channel();
        tx.send_group(vec![1, 2, 3]).unwrap();
        tx.send(4).unwrap();
        assert_eq!(rx.recv_timeout(Duration::ZERO).unwrap(), 1);
        assert_eq!(tx.diagnostics(), (3, 4));
        drop(rx);
        assert_eq!(tx.diagnostics(), (0, 4));
        assert!(tx.send(5).is_err());
        assert_eq!(tx.diagnostics(), (0, 4));
    }
    #[test]
    fn groups_stay_contiguous_between_concurrent_producers() {
        let (tx, mut rx) = channel();
        let workers: Vec<_> = (0..4)
            .map(|producer| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for index in 0..1000 {
                        tx.send_group(vec![
                            (producer, index, 0),
                            (producer, index, 1),
                            (producer, index, 2),
                        ])
                        .unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(tx.diagnostics(), (12_000, 12_000));
        for _ in 0..4000 {
            let a = rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let b = rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let c = rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(b, (a.0, a.1, 1));
            assert_eq!(c, (a.0, a.1, 2));
            assert_eq!(a.2, 0);
        }
        assert_eq!(tx.diagnostics(), (0, 12_000));
    }
    #[test]
    fn fifo_single_messages_failure_and_timeout_accounting() {
        let (tx, mut rx) = channel();
        assert!(tx.send_group(vec![]).is_err());
        assert!(tx.send_group(vec![0; 17]).is_err());
        tx.send(1).unwrap();
        tx.send_group(vec![2, 3]).unwrap();
        tx.send(4).unwrap();
        for expected in 1..=4 {
            assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), expected);
        }
        assert_eq!(
            rx.recv_timeout(Duration::ZERO),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        assert_eq!(tx.diagnostics().0, 0);
        drop(rx);
        assert!(tx.send_group(vec![5, 6, 7]).is_err());
        assert_eq!(tx.diagnostics().0, 0);
    }
}
