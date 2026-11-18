//! The write line: the jobs that wait, in the order the UI asked for them, and the one that runs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_channel::oneshot;
use gpui_kit::{AppContext as _, Context, Task};
use holm_core::Core;

use super::CoreDoor;

/// The work of one job. It runs off the UI thread, and gives the error of a failed job for the
/// log at quit.
pub(super) type Work = Box<dyn FnOnce(&Core) -> Option<String> + Send>;

/// The end of one job, on the UI thread after its work and its event: it gives the result to
/// the caller.
pub(super) type End = Box<dyn FnOnce(&mut CoreDoor, &mut Context<CoreDoor>)>;

/// One job of the line. Its number grows with each job.
struct Job {
    number: u64,
    work: Work,
    end: End,
}

/// The job whose work runs.
struct RunningJob {
    number: u64,
    end: End,
    /// It comes when the work ends. It is gone when the drain ran the work.
    done: oneshot::Receiver<()>,
    /// It ends the job when the work ends after its call.
    _wake: Task<()>,
}

/// The jobs of the user and the writes of the draft. One job runs at a time, and a failed job
/// does not stop the line.
pub(super) struct WriteLine {
    core: Core,
    jobs: VecDeque<Job>,
    next_number: u64,
    /// The number of the last job that ended.
    ended: u64,
    /// The last job that is past its budget, until it ends.
    late: Option<u64>,
    /// The work that left the line. Its thread holds the lock while it runs, so the drain
    /// waits for it, or runs it when the thread did not start yet.
    work: Arc<Mutex<Option<Work>>>,
    running: Option<RunningJob>,
    /// After the drain, no job runs.
    closed: bool,
}

impl WriteLine {
    pub(super) fn new(core: Core) -> Self {
        Self {
            core,
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
    pub(super) fn push(&mut self, work: Work, end: End) -> u64 {
        let number = self.next_number;
        self.next_number += 1;
        self.jobs.push_back(Job { number, work, end });
        number
    }

    /// Whether a job runs.
    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Whether a job is past its budget.
    pub(super) fn is_late(&self) -> bool {
        self.late.is_some()
    }

    /// Whether the job `number` ended.
    pub(super) fn has_ended(&self, number: u64) -> bool {
        self.ended >= number
    }

    /// Starts the first job, and gives whether it did. When its work ends after the call, the
    /// job ends in its own update, and the line runs on.
    pub(super) fn start(&mut self, cx: &mut Context<CoreDoor>) -> bool {
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
        let wake = cx.spawn(async move |door, cx| {
            task.await;
            door.update(cx, |door, cx| {
                let ran = door
                    .write_line
                    .running
                    .as_mut()
                    .is_some_and(|job| matches!(job.done.try_recv(), Ok(Some(()))));
                door.end_job(ran, cx);
                door.run(None, cx);
            })
            .ok();
        });
        self.running = Some(RunningJob {
            number,
            end,
            done,
            _wake: wake,
        });
        true
    }

    /// Waits on this thread up to `left` for the work of the running job, and gives whether it
    /// ran. When no job runs or the time is up, it gives `None`, and the job `until` is late.
    pub(super) fn wait(
        &mut self,
        until: u64,
        left: Duration,
        cx: &mut Context<CoreDoor>,
    ) -> Option<bool> {
        let done = match &mut self.running {
            Some(job) if !left.is_zero() => cx
                .foreground_executor()
                .block_with_timeout(left, &mut job.done)
                .ok(),
            _ => None,
        };
        if done.is_none() && self.late < Some(until) {
            self.late = Some(until);
            cx.notify();
        }
        done.map(|done| done.is_ok())
    }

    /// Ends the running job, and gives its end. A late job is no longer late once it ends.
    pub(super) fn end(&mut self, cx: &mut Context<CoreDoor>) -> Option<End> {
        let job = self.running.take()?;
        self.ended = job.number;
        if self.late.is_some_and(|late| late <= self.ended) {
            self.late = None;
            cx.notify();
        }
        Some(job.end)
    }

    /// Runs the work that left the line, every job that waits, and then `last`. Later jobs do
    /// not run. It works on the UI thread, because no task runs after the window is gone.
    pub(super) fn drain(&mut self, last: Option<Work>) {
        self.closed = true;
        let mut slot = lock(&self.work);
        let works = slot
            .take()
            .into_iter()
            .chain(self.jobs.drain(..).map(|job| job.work))
            .chain(last);
        for work in works {
            if let Some(failure) = work(&self.core) {
                tracing::warn!(error = %failure, "a job at quit failed");
            }
        }
    }
}

/// A lock that still works after a panic in another thread. A job never leaves a half-done
/// change on disk.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
