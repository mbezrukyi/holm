//! The write line: the jobs that wait, in the order the UI asked for them, and the one that runs.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use futures_channel::oneshot;
use futures_core::Stream;
use gpui_kit::{App, AppContext as _, Task};
use holm_core::model::ChangeList;
use holm_core::{ChangeLists, Core};

/// The work of one job. It runs off the UI thread, and gives the error of a failed job for the
/// log at quit.
pub(super) type Work = Box<dyn FnOnce(&Core) -> Option<String> + Send>;

/// A job whose work ran, with the change lists of the core since the job before it.
pub(super) struct Ended<E> {
    pub(super) end: E,
    pub(super) lists: Vec<ChangeList>,
}

/// What one [`WriteLine::run`] gives.
pub(super) struct Run<E> {
    /// The jobs whose work ran, in their order.
    pub(super) ended: Vec<Ended<E>>,
    /// It is there when the run started a job that still runs. It comes when the work of that
    /// job ends, and then the next run gives the job.
    pub(super) wake: Option<Wake>,
}

/// It comes when the work of a running job ends.
pub(super) struct Wake(Task<()>);

impl IntoFuture for Wake {
    type Output = ();
    type IntoFuture = Task<()>;

    fn into_future(self) -> Task<()> {
        self.0
    }
}

/// One job of the line. Its number grows with each job.
struct Job<E> {
    number: u64,
    work: Work,
    end: E,
}

/// The job whose work runs.
struct RunningJob<E> {
    number: u64,
    end: E,
    /// It comes when the work ends. It is gone when the drain ran the work.
    done: oneshot::Receiver<()>,
    /// The task of the work, until a run gives it as the [`Wake`].
    task: Option<Task<()>>,
}

/// The jobs of the user and the writes of the draft. One job runs at a time, and a failed job
/// does not stop the line. `E` is what the caller does at the end of a job.
pub(super) struct WriteLine<E> {
    core: Core,
    changes: ChangeLists,
    jobs: VecDeque<Job<E>>,
    next_number: u64,
    /// The number of the last job that ended.
    ended: u64,
    /// The last job that is past its budget, until it ends.
    late: Option<u64>,
    /// The work that left the line. Its thread holds the lock while it runs, so the drain
    /// waits for it, or runs it when the thread did not start yet.
    work: Arc<Mutex<Option<Work>>>,
    running: Option<RunningJob<E>>,
    /// After the drain, no job runs.
    closed: bool,
}

impl<E> WriteLine<E> {
    /// The line of `core`. `changes` are the change lists of `core`.
    pub(super) fn new(core: Core, changes: ChangeLists) -> Self {
        Self {
            core,
            changes,
            jobs: VecDeque::new(),
            next_number: 1,
            ended: 0,
            late: None,
            work: Arc::default(),
            running: None,
            closed: false,
        }
    }

    /// Puts a job at the end of the line, and gives its number.
    pub(super) fn push(&mut self, work: Work, end: E) -> u64 {
        let number = self.next_number;
        self.next_number += 1;
        self.jobs.push_back(Job { number, work, end });
        number
    }

    /// Whether a job is past its budget.
    pub(super) fn is_late(&self) -> bool {
        self.late.is_some()
    }

    /// Runs the jobs, one at a time, and gives the jobs that ended. With `until`, it waits on
    /// this thread up to `budget` until the job `until` ended. When the time is up, that job
    /// is late until it ends. Without `until`, it only starts the next job.
    pub(super) fn run(&mut self, until: Option<u64>, budget: Duration, cx: &App) -> Run<E> {
        let deadline = Instant::now() + budget;
        let mut ended = Vec::new();
        loop {
            if self.running.is_none() && !self.start(cx) {
                break;
            }
            let wait = until
                .filter(|until| self.ended < *until)
                .map(|_| deadline.saturating_duration_since(Instant::now()));
            let Some(ran) = self.wait(wait, cx) else {
                if let Some(until) = until
                    && self.ended < until
                    && self.late < Some(until)
                {
                    self.late = Some(until);
                }
                break;
            };
            if let Some(job) = self.end(ran) {
                ended.push(job);
            }
        }
        let wake = self.running.as_mut().and_then(|job| job.task.take()).map(Wake);
        Run { ended, wake }
    }

    /// The change lists that wait, while no job runs. During a job it gives `None`, and the
    /// lists wait for the end of the job. It wakes `waker` when a list comes.
    pub(super) fn outside(&mut self, waker: &mut task::Context<'_>) -> Option<Vec<ChangeList>> {
        if self.running.is_some() {
            return None;
        }
        let mut lists = Vec::new();
        while let Poll::Ready(Some(list)) = Pin::new(&mut self.changes).poll_next(waker) {
            lists.push(list);
        }
        Some(lists)
    }

    /// Runs the work that left the line, every job that waits, and then `last`. Later jobs do
    /// not run. It works on the UI thread, because no task runs after the window is gone.
    pub(super) fn drain(&mut self, last: Work) {
        self.closed = true;
        let mut slot = lock(&self.work);
        let works = slot
            .take()
            .into_iter()
            .chain(self.jobs.drain(..).map(|job| job.work))
            .chain(std::iter::once(last));
        for work in works {
            if let Some(failure) = work(&self.core) {
                tracing::warn!(error = %failure, "a job at quit failed");
            }
        }
    }

    /// Starts the first job, and gives whether it did.
    fn start(&mut self, cx: &App) -> bool {
        if self.closed {
            return false;
        }
        let Some(Job { number, work, end }) = self.jobs.pop_front() else {
            return false;
        };
        *lock(&self.work) = Some(work);
        let core = self.core.clone();
        let slot = self.work.clone();
        let (send, done) = oneshot::channel();
        let task = cx.background_spawn(async move {
            let mut slot = lock(&slot);
            if let Some(work) = slot.take() {
                work(&core);
                send.send(()).ok();
            }
        });
        self.running = Some(RunningJob {
            number,
            end,
            done,
            task: Some(task),
        });
        true
    }

    /// Whether the work of the running job ended, and whether it ran. With `wait`, it waits
    /// on this thread up to that time. It gives `None` while the work runs.
    fn wait(&mut self, wait: Option<Duration>, cx: &App) -> Option<bool> {
        let job = self.running.as_mut()?;
        let done = match job.done.try_recv() {
            Ok(None) => match wait {
                Some(left) if !left.is_zero() => {
                    cx.foreground_executor().block_with_timeout(left, &mut job.done).ok()?
                }
                _ => return None,
            },
            Ok(Some(())) => Ok(()),
            Err(canceled) => Err(canceled),
        };
        Some(done.is_ok())
    }

    /// Ends the running job. When its work `ran`, it gives the job with the lists that wait.
    /// A late job is no longer late once it ends.
    fn end(&mut self, ran: bool) -> Option<Ended<E>> {
        let job = self.running.take()?;
        self.ended = job.number;
        if self.late.is_some_and(|late| late <= self.ended) {
            self.late = None;
        }
        ran.then(|| Ended {
            end: job.end,
            lists: std::iter::from_fn(|| self.changes.try_next()).collect(),
        })
    }
}

/// A lock that still works after a panic in another thread. A job never leaves a half-done
/// change on disk.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::cell::Cell;
    use std::rc::Rc;
    use std::task::Waker;

    use gpui_kit::TestAppContext;
    use holm_core::FolderPath;
    use tempfile::TempDir;

    use super::*;

    fn line() -> (TempDir, WriteLine<u32>) {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
        let (_, changes) = core.follow();
        (dir, WriteLine::new(core, changes))
    }

    fn folder(name: &'static str) -> Work {
        Box::new(move |core| {
            core.create_folder(&FolderPath::root(), name)
                .err()
                .map(|error| error.to_string())
        })
    }

    fn ends(run: &Run<u32>) -> Vec<(u32, usize)> {
        run.ended.iter().map(|job| (job.end, job.lists.len())).collect()
    }

    #[gpui_kit::test]
    fn the_jobs_in_the_budget_end_in_order_with_their_lists(cx: &mut TestAppContext) {
        let (_dir, mut line) = line();
        line.push(folder("B"), 1);
        line.push(Box::new(|_| None), 2);
        let last = line.push(folder("C"), 3);

        let run = cx.update(|cx| line.run(Some(last), Duration::from_secs(10), cx));

        assert_eq!(ends(&run), [(1, 1), (2, 0), (3, 1)]);
        assert!(run.wake.is_none());
        assert!(!line.is_late());
    }

    #[gpui_kit::test]
    fn a_job_past_its_budget_is_late_and_its_wake_lets_the_next_run_end_it(cx: &mut TestAppContext) {
        let (_dir, mut line) = line();
        let number = line.push(folder("B"), 1);
        let run = cx.update(|cx| line.run(Some(number), Duration::ZERO, cx));
        assert!(run.ended.is_empty());
        assert!(line.is_late());
        let mut waker = task::Context::from_waker(Waker::noop());
        assert!(line.outside(&mut waker).is_none());

        let seen = Rc::new(Cell::new(false));
        let wake = run.wake.unwrap();
        cx.update(|cx| {
            let seen = seen.clone();
            cx.spawn(async move |_| {
                wake.await;
                seen.set(true);
            })
            .detach();
        });
        cx.run_until_parked();
        assert!(seen.get());

        let run = cx.update(|cx| line.run(None, Duration::ZERO, cx));
        assert_eq!(ends(&run), [(1, 1)]);
        assert!(run.wake.is_none());
        assert!(!line.is_late());
        assert_eq!(line.outside(&mut waker).map(|lists| lists.len()), Some(0));
    }

    #[gpui_kit::test]
    fn the_drain_runs_the_jobs_and_then_the_line_gives_no_end(cx: &mut TestAppContext) {
        let (dir, mut line) = line();
        let number = line.push(folder("B"), 1);
        cx.update(|cx| line.run(Some(number), Duration::ZERO, cx));
        line.push(folder("C"), 2);

        line.drain(folder("D"));
        let later = line.push(folder("E"), 3);
        cx.run_until_parked();
        let run = cx.update(|cx| line.run(Some(later), Duration::from_secs(10), cx));

        assert!(run.ended.is_empty());
        assert!(run.wake.is_none());
        let notes = dir.path().join("notes");
        for name in ["B", "C", "D"] {
            assert!(notes.join(name).is_dir(), "{name}");
        }
        assert!(!notes.join("E").exists());
    }
}
