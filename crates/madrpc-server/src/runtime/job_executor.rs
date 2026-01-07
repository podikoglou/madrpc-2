//! Tokio job executor for Boa's promise job queue
//!
//! This module implements a custom `JobExecutor` that integrates Boa's promise
//! job queue with tokio's async runtime. This is essential for supporting
//! JavaScript promises and async functions within MaDRPC.
//!
//! # Architecture
//!
//! Boa's JavaScript engine uses a job queue to manage promise resolution and
//! async operations. This executor:
//!
//! 1. Stores jobs in separate queues (promise, async, generic)
//! 2. Provides async execution via `run_jobs_async()`
//! 3. Integrates with tokio's runtime for concurrent async job execution
//!
//! # Job Types
//!
//! - **PromiseJob**: Microtasks for promise resolution (then/catch handlers)
//! - **AsyncJob**: Native async jobs created by `NativeFunction::from_async_fn`
//! - **GenericJob**: General-purpose jobs
//!
//! # Execution Model
//!
//! When `run_jobs_async()` is called:
//! 1. All pending async jobs are polled concurrently using `FutureGroup`
//! 2. After each async job completes, promise and generic jobs are drained
//! 3. The loop continues until all queues are empty

use boa_engine::{context::Context, job::{GenericJob, Job, JobExecutor, NativeAsyncJob, PromiseJob}};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// A JobExecutor that integrates Boa's promise job queue with tokio's async runtime.
///
/// This executor stores pending promise jobs in separate queues and provides an async
/// method to process them, allowing JavaScript promises to be resolved within
/// an async context.
///
/// # Thread Safety
///
/// This executor uses `Rc<RefCell<_>>` for interior mutability, which is safe
/// because Boa's job executor is designed to be used from a single thread.
///
/// # Example
///
/// ```ignore
/// let executor = Rc::new(TokioJobExecutor::new());
/// let mut ctx = Context::builder()
///     .job_executor(executor.clone())
///     .build()?;
///
/// // Run any pending jobs
/// executor.run_jobs_async(&RefCell::new(&mut ctx)).await?;
/// ```
pub struct TokioJobExecutor {
    /// Queue for promise microtasks (then/catch handlers)
    promise_jobs: RefCell<VecDeque<PromiseJob>>,
    /// Queue for native async jobs
    async_jobs: RefCell<VecDeque<NativeAsyncJob>>,
    /// Queue for general-purpose jobs
    generic_jobs: RefCell<VecDeque<GenericJob>>,
}

impl TokioJobExecutor {
    /// Creates a new TokioJobExecutor with empty job queues.
    pub fn new() -> Self {
        Self {
            promise_jobs: RefCell::default(),
            async_jobs: RefCell::default(),
            generic_jobs: RefCell::default(),
        }
    }

    /// Returns true if there are pending jobs in any queue.
    ///
    /// This can be used to check if `run_jobs_async()` would do any work
    /// before calling it.
    ///
    /// # Returns
    ///
    /// `true` if any of the three job queues (promise, async, or generic) contain
    /// pending jobs; `false` otherwise.
    pub fn has_pending_jobs(&self) -> bool {
        !self.promise_jobs.borrow().is_empty()
            || !self.async_jobs.borrow().is_empty()
            || !self.generic_jobs.borrow().is_empty()
    }

    /// Drains all pending jobs (except async jobs which are polled separately).
    ///
    /// This method:
    /// 1. Runs at most one generic job (macrotask semantics)
    /// 2. Runs all pending promise jobs (microtask semantics)
    /// 3. Clears Boa's kept objects cache
    ///
    /// # Parameters
    ///
    /// * `context` - Mutable reference to the Boa context for job execution
    ///
    /// # Note
    ///
    /// Async jobs are NOT drained here; they must be polled separately via
    /// `run_jobs_async()`.
    fn drain_jobs(&self, context: &mut Context) {
        // Run one generic job if available
        if let Some(generic) = self.generic_jobs.borrow_mut().pop_front() {
            if let Err(err) = generic.call(context) {
                tracing::error!("Uncaught error in generic job: {err}");
            }
        }

        // Run all promise jobs
        let jobs = std::mem::take(&mut *self.promise_jobs.borrow_mut());
        for job in jobs {
            if let Err(e) = job.call(context) {
                tracing::error!("Uncaught error in promise job: {e}");
            }
        }

        context.clear_kept_objects();
    }
}

impl Default for TokioJobExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JobExecutor for TokioJobExecutor {
    /// Enqueues a job to be executed later.
    ///
    /// This is called by Boa's promise implementation when a promise
    /// resolution job needs to be scheduled. Jobs are stored in separate
    /// queues based on their type.
    ///
    /// # Parameters
    ///
    /// * `job` - The job to enqueue (PromiseJob, AsyncJob, or GenericJob)
    /// * `_context` - Unused parameter (required by trait)
    ///
    /// # Note
    ///
    /// TimeoutJob and other job types are not supported and will be logged
    /// as warnings.
    fn enqueue_job(self: Rc<Self>, job: Job, _context: &mut Context) {
        match job {
            Job::PromiseJob(job) => self.promise_jobs.borrow_mut().push_back(job),
            Job::AsyncJob(job) => self.async_jobs.borrow_mut().push_back(job),
            Job::GenericJob(g) => self.generic_jobs.borrow_mut().push_back(g),
            _ => {
                // TimeoutJob and other types not supported in this simplified version
                tracing::warn!("Unsupported job type enqueued, ignoring");
            }
        }
    }

    /// Runs all jobs synchronously (blocking).
    ///
    /// This creates a tokio runtime if one doesn't exist, or uses the existing one.
    /// If already inside a tokio runtime, it uses `block_in_place` to avoid
    /// nested runtime issues.
    ///
    /// # Parameters
    ///
    /// * `context` - Mutable reference to the Boa context
    ///
    /// # Returns
    ///
    /// `Ok(())` if all jobs ran successfully, or an error if a job failed.
    ///
    /// # Blocking Behavior
    ///
    /// This function will block the current thread until all jobs complete.
    /// It should only be called when async execution is not possible.
    fn run_jobs(self: Rc<Self>, context: &mut Context) -> boa_engine::JsResult<()> {
        // Try to use the current runtime if we're in one
        if tokio::runtime::Handle::try_current().is_ok() {
            // We're inside a tokio runtime, use block_in_place to run synchronously
            return tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    // Create a LocalSet for this scope
                    let local_set = tokio::task::LocalSet::new();
                    local_set.run_until(self.run_jobs_async(&RefCell::new(context))).await
                })
            });
        }

        // No current runtime, create a new one
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        tokio::task::LocalSet::new()
            .block_on(&runtime, self.run_jobs_async(&RefCell::new(context)))
    }

    /// Runs all jobs asynchronously.
    ///
    /// This processes async jobs concurrently while draining promise and
    /// generic jobs after each async job completes.
    ///
    /// # Execution Algorithm
    ///
    /// 1. Poll all pending async jobs concurrently using `FutureGroup`
    /// 2. Wait for at least one async job to complete
    /// 3. Drain all promise and generic jobs (microtasks)
    /// 4. Yield back to the tokio scheduler
    /// 5. Repeat until all queues are empty
    ///
    /// # Parameters
    ///
    /// * `context` - RefCell-wrapped mutable reference to the Boa context
    ///
    /// # Returns
    ///
    /// `Ok(())` when all jobs have completed, or an error if a job failed.
    ///
    /// # Concurrency
    ///
    /// Multiple async jobs are polled concurrently, but promise jobs are
    /// executed sequentially between async job completions to maintain
    /// proper microtask semantics.
    async fn run_jobs_async(self: Rc<Self>, context: &RefCell<&mut Context>) -> boa_engine::JsResult<()>
    where
        Self: Sized,
    {
        use futures_concurrency::future::FutureGroup;
        use futures_lite::{future, StreamExt};

        let mut group = FutureGroup::new();

        loop {
            // Poll all async jobs
            for job in std::mem::take(&mut *self.async_jobs.borrow_mut()) {
                group.insert(job.call(context));
            }

            // Check if all queues are empty and no async jobs pending
            if group.is_empty()
                && self.promise_jobs.borrow().is_empty()
                && self.generic_jobs.borrow().is_empty()
            {
                return Ok(());
            }

            // Poll pending async tasks once
            if let Some(Err(err)) = future::poll_once(group.next()).await.flatten() {
                tracing::error!("Uncaught error in async job: {err}");
            }

            // Drain microtasks and run one macrotask
            self.drain_jobs(&mut context.borrow_mut());
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_default_creates_executor() {
        let executor = TokioJobExecutor::default();
        assert!(!executor.has_pending_jobs());
    }

    #[test]
    fn test_new_creates_executor() {
        let executor = TokioJobExecutor::new();
        assert!(!executor.has_pending_jobs());
    }

    #[test]
    fn test_has_pending_jobs_false_when_empty() {
        let executor = TokioJobExecutor::new();
        assert!(!executor.has_pending_jobs());
    }

    #[tokio::test]
    async fn test_run_jobs_on_empty_context() {
        let executor = Rc::new(TokioJobExecutor::new());
        let mut context = Context::default();

        // Running on empty queues should complete immediately
        let result = executor.clone().run_jobs_async(&RefCell::new(&mut context)).await;
        assert!(result.is_ok());
        assert!(!executor.has_pending_jobs());
    }

    #[tokio::test]
    async fn test_enqueue_and_run_generic_job() {
        let executor = Rc::new(TokioJobExecutor::new());
        let mut context = Context::default();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        // Enqueue a simple job via the JobExecutor trait
        let realm = context.realm().clone();
        let job = Job::GenericJob(GenericJob::new(
            move |_| {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(boa_engine::JsValue::undefined())
            },
            realm,
        ));

        executor.clone().enqueue_job(job, &mut context);

        assert!(executor.has_pending_jobs());

        // Run the job
        executor.clone().run_jobs_async(&RefCell::new(&mut context)).await.unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(!executor.has_pending_jobs());
    }

    #[tokio::test]
    async fn test_run_jobs_multiple_generic_jobs() {
        let executor = Rc::new(TokioJobExecutor::new());
        let mut context = Context::default();

        let counter = Arc::new(AtomicUsize::new(0));

        // Enqueue multiple generic jobs
        for _ in 0..5 {
            let counter_clone = Arc::clone(&counter);
            let realm = context.realm().clone();
            let job = Job::GenericJob(GenericJob::new(
                move |_| {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    Ok(boa_engine::JsValue::undefined())
                },
                realm,
            ));
            executor.clone().enqueue_job(job, &mut context);
        }

        assert!(executor.has_pending_jobs());

        // Run all jobs
        executor.clone().run_jobs_async(&RefCell::new(&mut context)).await.unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 5);
        assert!(!executor.has_pending_jobs());
    }

    #[test]
    fn test_drain_jobs_empty() {
        let executor = TokioJobExecutor::new();
        let mut context = Context::default();

        // Should not panic on empty queue
        executor.drain_jobs(&mut context);
    }
}
