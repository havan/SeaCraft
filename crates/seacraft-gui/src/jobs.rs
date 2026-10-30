//! Background jobs and the revision stamps of their results.
//!
//! Every analysis result in the app is computed for a *revision*: a key
//! derived from the inputs it depends on (the model, the floating condition,
//! the project, …). A [`Job`] runs one computation on a plain std thread and
//! carries the revision it was started for. A [`Track`] is the bookkeeping
//! beside one result: the revision the shown result was computed for, the
//! job computing the next one, and the last failure.
//!
//! The rules the app relies on:
//!
//! * a worker panic is caught and becomes [`Poll::Failed`] with the panic
//!   message — never a silently dropped worker;
//! * starting a job supersedes the running one: the old result is discarded
//!   (its thread runs to the end, its answer goes nowhere);
//! * a job that finishes for a revision that is no longer current is
//!   discarded too, so a result for an edited hull is never taken as fresh;
//! * a failure is remembered with its revision, so an automatic recompute
//!   does not retry the same failing inputs every frame.

use std::sync::mpsc;

/// What a [`Job`] reports when polled.
#[derive(Debug, PartialEq)]
pub enum Poll<T> {
    /// Still working.
    Running,
    /// Finished with a result.
    Done(T),
    /// Returned an error or panicked; the message says why.
    Failed(String),
}

/// One computation on a background thread, stamped with the revision it was
/// started for.
pub struct Job<T> {
    revision: u64,
    rx: mpsc::Receiver<Result<T, String>>,
}

impl<T: Send + 'static> Job<T> {
    /// Run `work` on a new thread. An `Err` or a panic in `work` becomes
    /// [`Poll::Failed`].
    pub fn spawn(revision: u64, work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|payload| Err(panic_message(payload.as_ref())));
            // The receiver is gone when the job was superseded: nobody wants
            // the answer any more.
            let _ = tx.send(out);
        });
        Self { revision, rx }
    }
}

impl<T> Job<T> {
    /// The revision this job computes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Without blocking: still running, or the result. A job reports its
    /// result once; poll it no more after `Done` or `Failed`.
    pub fn poll(&self) -> Poll<T> {
        match self.rx.try_recv() {
            Ok(Ok(v)) => Poll::Done(v),
            Ok(Err(e)) => Poll::Failed(e),
            Err(mpsc::TryRecvError::Empty) => Poll::Running,
            Err(mpsc::TryRecvError::Disconnected) => Poll::Failed(STOPPED.to_owned()),
        }
    }

    /// Block until the job ends.
    pub fn wait(self) -> Result<T, String> {
        self.rx.recv().unwrap_or_else(|_| Err(STOPPED.to_owned()))
    }
}

const STOPPED: &str = "the worker stopped without a result";

/// The text of a panic payload (`panic!` with a literal or a format string).
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    format!("worker panicked: {text}")
}

/// The revision stamp of one result, the job computing the next one and the
/// last failure. The result itself lives with the rest of its state.
pub struct Track<T> {
    /// The revision the stored result was computed for.
    stamp: Option<u64>,
    /// The last failure and the revision it failed for.
    failure: Option<(u64, String)>,
    job: Option<Job<T>>,
}

impl<T> Default for Track<T> {
    fn default() -> Self {
        Self {
            stamp: None,
            failure: None,
            job: None,
        }
    }
}

impl<T: Send + 'static> Track<T> {
    /// Start computing `revision`, superseding any running job.
    pub fn start(
        &mut self,
        revision: u64,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) {
        self.job = Some(Job::spawn(revision, work));
    }
}

impl<T> Track<T> {
    /// A result for `revision` was stored by the caller (a synchronous
    /// compute): any running job is superseded and old failures forgotten.
    pub fn stamp(&mut self, revision: u64) {
        self.stamp = Some(revision);
        self.failure = None;
        self.job = None;
    }

    /// Computing `revision` failed before a job could start (invalid
    /// input): remembered like a job's failure.
    pub fn fail(&mut self, revision: u64, message: String) {
        self.failure = Some((revision, message));
        self.job = None;
    }

    /// Forget everything: no result, no job, no failure.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Collect a finished job. `Some(Ok)` is a result for `current` (now
    /// stamped; the caller stores it), `Some(Err)` its failure (remembered).
    /// A job for any other revision is discarded and `None` returned.
    pub fn poll(&mut self, current: u64) -> Option<Result<T, String>> {
        let job = self.job.as_ref()?;
        let out = match job.poll() {
            Poll::Running => return None,
            Poll::Done(v) => Ok(v),
            Poll::Failed(e) => Err(e),
        };
        let revision = job.revision();
        self.job = None;
        self.finish(revision, current, out)
    }

    /// Block until the running job ends, then as [`Track::poll`].
    pub fn wait(&mut self, current: u64) -> Option<Result<T, String>> {
        let job = self.job.take()?;
        let revision = job.revision();
        let out = job.wait();
        self.finish(revision, current, out)
    }

    fn finish(
        &mut self,
        revision: u64,
        current: u64,
        out: Result<T, String>,
    ) -> Option<Result<T, String>> {
        if revision != current {
            return None;
        }
        match &out {
            Ok(_) => {
                self.stamp = Some(revision);
                self.failure = None;
            }
            Err(e) => self.failure = Some((revision, e.clone())),
        }
        Some(out)
    }

    /// The stored result was computed for `current`.
    pub fn is_current(&self, current: u64) -> bool {
        self.stamp == Some(current)
    }

    /// A job is running (perhaps for an old revision, about to be discarded).
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The revision the running job computes.
    pub fn running_for(&self) -> Option<u64> {
        self.job.as_ref().map(Job::revision)
    }

    /// Why computing `current` failed, if it did.
    pub fn failure(&self, current: u64) -> Option<&str> {
        match &self.failure {
            Some((r, e)) if *r == current => Some(e),
            _ => None,
        }
    }

    /// `current` still needs computing: no result for it, no job on it and
    /// no failure for it.
    pub fn due(&self, current: u64) -> bool {
        !self.is_current(current)
            && self.running_for() != Some(current)
            && self.failure(current).is_none()
    }
}

/// One revision key from the counters a result depends on.
pub fn revision_key(parts: &[u64]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn settle<T>(job: &Job<T>) -> Poll<T> {
        let t0 = Instant::now();
        loop {
            match job.poll() {
                Poll::Running if t0.elapsed() < Duration::from_secs(60) => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                other => return other,
            }
        }
    }

    #[test]
    fn a_job_reports_its_result() {
        let job = Job::spawn(7, || Ok(6 * 7));
        assert_eq!(job.revision(), 7);
        assert_eq!(settle(&job), Poll::Done(42));
    }

    #[test]
    fn a_panicking_job_fails_with_its_message() {
        let job: Job<u32> = Job::spawn(1, || panic!("the solver diverged at 40°"));
        match settle(&job) {
            Poll::Failed(msg) => assert!(msg.contains("the solver diverged at 40°"), "{msg}"),
            other => panic!("expected a failure, got {other:?}"),
        }
        let job: Job<u32> = Job::spawn(1, || panic!("formatted {}", 3));
        assert!(matches!(job.wait(), Err(m) if m.contains("formatted 3")));
    }

    #[test]
    fn an_error_is_a_failure() {
        let job: Job<u32> = Job::spawn(1, || Err("no loading condition".into()));
        assert_eq!(settle(&job), Poll::Failed("no loading condition".into()));
    }

    #[test]
    fn a_superseded_job_is_discarded() {
        let mut track: Track<&'static str> = Track::default();
        let (release, gate) = mpsc::channel::<()>();
        // The old job finishes only after the new one.
        track.start(1, move || {
            let _ = gate.recv();
            Ok("old")
        });
        track.start(2, || Ok("new"));
        let _ = release.send(());
        assert_eq!(track.wait(2), Some(Ok("new")));
        assert!(track.is_current(2));
        assert!(!track.busy());
    }

    #[test]
    fn a_result_for_an_old_revision_is_discarded() {
        let mut track: Track<u32> = Track::default();
        track.start(1, || Ok(1));
        // The inputs moved on to revision 2 while the job ran.
        assert_eq!(track.wait(2), None);
        assert!(!track.is_current(1) && !track.is_current(2));
        assert!(track.due(2));
    }

    #[test]
    fn a_failure_is_not_retried_for_the_same_revision() {
        let mut track: Track<u32> = Track::default();
        track.start(3, || panic!("boom"));
        let out = track.wait(3).expect("a failure for revision 3");
        assert!(out.unwrap_err().contains("boom"));
        assert_eq!(track.failure(3).map(|m| m.contains("boom")), Some(true));
        assert!(!track.due(3), "a failed revision is not retried");
        assert!(track.due(4), "new inputs are tried again");
        track.stamp(4);
        assert!(track.failure(3).is_none() && track.is_current(4));
    }

    #[test]
    fn revision_keys_differ_with_their_parts() {
        assert_eq!(revision_key(&[1, 2, 3]), revision_key(&[1, 2, 3]));
        assert_ne!(revision_key(&[1, 2, 3]), revision_key(&[1, 3, 2]));
        assert_ne!(revision_key(&[1, 2]), revision_key(&[1, 2, 0]));
    }
}
