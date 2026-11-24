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

/// The work of one job. It runs off the UI thread, and gives the error of a failed job for the log at quit.
pub(super) type Work = Box<dyn FnOnce(&Core) -> Option<String> + Send>;

/// What one [`WriteLine::run`] gives.
pub(super) struct Run<E> {
    /// The ends of the jobs whose work ran, in their order.
    pub(super) ended: Vec<E>,
    /// The change lists of the core up to the last job that ended.
    pub(super) lists: Vec<ChangeList>,
    /// It is there when the run started a job that still runs. It comes when the work of that job ends, and then the
    /// next run gives the job.
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

/// It grows with each job.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct JobNumber(u64);

impl JobNumber {
    fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

struct Job<E> {
    number: JobNumber,
    work: Work,
    end: E,
}

struct RunningJob<E> {
    number: JobNumber,
    end: E,
    /// It comes when the work ends. It is gone when the drain ran the work.
    done: oneshot::Receiver<()>,
    /// The task of the work, until a run gives it as the [`Wake`].
    task: Option<Task<()>>,
}

/// The jobs of the user and the writes of the draft. One job runs at a time, and a failed job does not stop the line.
/// `E` is what the caller does at the end of a job.
pub(super) struct WriteLine<E> {
    core: Core,
    changes: ChangeLists,
    jobs: VecDeque<Job<E>>,
    next_number: JobNumber,
    last_ended: Option<JobNumber>,
    /// The last job that is past its budget, until it ends.
    late: Option<JobNumber>,
    /// The work that left the line. Its thread holds the lock while it runs, so the drain waits for it, or runs it
    /// when the thread did not start yet.
    work: Arc<Mutex<Option<Work>>>,
    running: Option<RunningJob<E>>,
    /// After the drain, no job runs.
    closed: bool,
}

impl<E> WriteLine<E> {
    /// `changes` are the change lists of `core`.
    pub(super) fn new(core: Core, changes: ChangeLists) -> Self {
        Self {
            core,
            changes,
            jobs: VecDeque::new(),
            next_number: JobNumber::default(),
            last_ended: None,
            late: None,
            work: Arc::default(),
            running: None,
            closed: false,
        }
    }

    pub(super) fn push(&mut self, work: Work, end: E) -> JobNumber {
        let number = self.next_number;
        self.next_number = number.next();
        self.jobs.push_back(Job { number, work, end });
        number
    }

    /// Whether a job is past its budget.
    pub(super) fn is_late(&self) -> bool {
        self.late.is_some()
    }

    /// Runs the jobs, one at a time, and gives the jobs that ended. With `until`, it waits on this thread up to
    /// `budget` until the job `until` ended. When the time is up, that job is late until it ends. Without `until`, it
    /// only starts the next job.
    pub(super) fn run(&mut self, until: Option<JobNumber>, budget: Duration, cx: &App) -> Run<E> {
        let deadline = Instant::now() + budget;
        let mut ended = Vec::new();
        let mut lists = Vec::new();
        loop {
            if self.running.is_none() && !self.start(cx) {
                break;
            }
            let wait = until
                .filter(|until| self.last_ended < Some(*until))
                .map(|_| deadline.saturating_duration_since(Instant::now()));
            let Some(ran) = self.wait(wait, cx) else {
                if let Some(until) = until
                    && self.last_ended < Some(until)
                    && self.late < Some(until)
                {
                    self.late = Some(until);
                }
                break;
            };
            if let Some(end) = self.end(ran) {
                ended.push(end);
                lists.extend(std::iter::from_fn(|| self.changes.try_next()));
            }
        }
        let wake = self.running.as_mut().and_then(|job| job.task.take()).map(Wake);
        Run { ended, lists, wake }
    }

    /// The change lists that wait, while no job runs. During a job it gives `None`, and the lists wait for the end of
    /// the job. It wakes `waker` when a list comes.
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

    /// Runs the work that left the line, every job that waits, and then `last`. Later jobs do not run. It works on the
    /// UI thread, because no task runs after the window is gone.
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

    /// Whether the work of the running job ended: `Ok` when it ran, `Canceled` when it never ran. With `wait`, it waits
    /// on this thread up to that time. It gives `None` while the work runs.
    fn wait(&mut self, wait: Option<Duration>, cx: &App) -> Option<Result<(), oneshot::Canceled>> {
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
        Some(done)
    }

    /// Ends the running job. When its work ran, it gives the end of the job. A late job is no longer late once it ends.
    fn end(&mut self, ran: Result<(), oneshot::Canceled>) -> Option<E> {
        let job = self.running.take()?;
        self.last_ended = Some(job.number);
        if self.late.is_some_and(|late| late <= job.number) {
            self.late = None;
        }
        ran.ok().map(|()| job.end)
    }
}

/// A lock that still works after a panic in another thread. A job never leaves a half-done change on disk.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
