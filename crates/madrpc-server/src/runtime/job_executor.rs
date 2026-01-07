use boa_engine::{context::Context, job::{GenericJob, Job, JobExecutor, NativeAsyncJob, PromiseJob}};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// A JobExecutor that integrates Boa's promise job queue with tokio's async runtime.
///
/// This executor stores pending promise jobs in separate queues and provides an async
/// method to process them, allowing JavaScript promises to be resolved within
/// an async context.
pub struct TokioJobExecutor {
    promise_jobs: RefCell<VecDeque<PromiseJob>>,
    async_jobs: RefCell<VecDeque<NativeAsyncJob>>,
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
    pub fn has_pending_jobs(&self) -> bool {
        !self.promise_jobs.borrow().is_empty()
            || !self.async_jobs.borrow().is_empty()
            || !self.generic_jobs.borrow().is_empty()
    }

    /// Drains all pending jobs (except async jobs which are polled separately).
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
    /// resolution job needs to be scheduled.
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
    /// This creates a tokio runtime and blocks until all jobs are complete.
    fn run_jobs(self: Rc<Self>, context: &mut Context) -> boa_engine::JsResult<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        tokio::task::LocalSet::default()
            .block_on(&runtime, self.run_jobs_async(&RefCell::new(context)))
    }

    /// Runs all jobs asynchronously.
    ///
    /// This processes async jobs concurrently while draining promise and
    /// generic jobs after each async job completes.
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
