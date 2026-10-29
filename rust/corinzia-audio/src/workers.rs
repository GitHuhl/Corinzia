//! Realtime worker threads for the engine: each block, the instruments
//! (each with its inserts) render in parallel, the audio thread taking a
//! share of them too.
//!
//! No locks and no allocation per block: the audio thread publishes the
//! block's jobs, wakes the workers (a futex wake on Linux), claims jobs
//! itself and spins until the last claimed one is finished. Jobs are
//! claimed from one atomic word holding (block, count, next), so a worker
//! late from one block can never take a job of the next.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// One job of a block: `ctx` is the block's context, `worker` indexes the
/// scratch of the thread running it (0 is the audio thread).
pub type JobFn = unsafe fn(ctx: *const (), job: usize, worker: usize);

/// At most this many jobs in a block (they are counted in 16 bits).
pub const MAX_JOBS: usize = 0xffff;

struct Shared {
    /// block << 32 | count << 16 | next.
    state: AtomicU64,
    /// The block's job; written before `state` is published, read after a
    /// claim, replaced only once every job of the block is done.
    job: UnsafeCell<Option<(JobFn, *const ())>>,
    done: AtomicU32,
    quit: AtomicBool,
}
unsafe impl Sync for Shared {}
unsafe impl Send for Shared {}

impl Shared {
    /// Claims and runs jobs of `block` until none is left.
    fn drain(&self, block: u32, worker: usize) {
        loop {
            let s = self.state.load(Ordering::Acquire);
            let (count, next) = ((s >> 16) & 0xffff, s & 0xffff);
            if (s >> 32) as u32 != block || next >= count {
                return;
            }
            if self.state.compare_exchange_weak(s, s + 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
                continue;
            }
            // The claim keeps the block from finishing: the job stays valid.
            if let Some((f, ctx)) = unsafe { *self.job.get() } {
                unsafe { f(ctx, next as usize, worker) };
            }
            self.done.fetch_add(1, Ordering::Release);
        }
    }
}

pub struct Workers {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
    block: u32,
    prioritized: bool,
}

impl Workers {
    /// `n` worker threads (0: the audio thread renders everything).
    pub fn new(n: usize) -> Workers {
        let shared = Arc::new(Shared { state: AtomicU64::new(0), job: UnsafeCell::new(None), done: AtomicU32::new(0), quit: AtomicBool::new(false) });
        let handles = (0..n)
            .filter_map(|i| {
                let sh = shared.clone();
                std::thread::Builder::new().name(format!("corinzia-render-{}", i + 1)).spawn(move || worker(sh, i + 1)).ok()
            })
            .collect();
        Workers { shared, handles, block: 0, prioritized: false }
    }

    /// One worker per core beside the audio thread's.
    pub fn for_this_machine() -> Workers {
        Workers::new(std::thread::available_parallelism().map(|n| n.get() - 1).unwrap_or(0).min(31))
    }

    /// Threads that run jobs, the audio thread included.
    pub fn threads(&self) -> usize {
        self.handles.len() + 1
    }

    /// Runs `count` jobs and returns when all are finished. Audio thread.
    ///
    /// # Safety
    /// `f(ctx, job, worker)` must be sound to run concurrently for distinct
    /// jobs, from any of the threads, until this returns.
    pub unsafe fn run(&mut self, count: usize, f: JobFn, ctx: *const ()) {
        let count = count.min(MAX_JOBS);
        if count == 0 {
            return;
        }
        if count == 1 || self.handles.is_empty() {
            for j in 0..count {
                f(ctx, j, 0);
            }
            return;
        }
        self.match_priority();
        let sh = &*self.shared;
        *sh.job.get() = Some((f, ctx));
        sh.done.store(0, Ordering::Relaxed);
        self.block = self.block.wrapping_add(1);
        sh.state.store((self.block as u64) << 32 | (count as u64) << 16, Ordering::Release);
        for h in self.handles.iter().take(count - 1) {
            h.thread().unpark();
        }
        sh.drain(self.block, 0);
        while sh.done.load(Ordering::Acquire) < count as u32 {
            std::hint::spin_loop();
        }
        *sh.job.get() = None;
    }

    /// Gives the workers the audio thread's scheduling (realtime under
    /// PipeWire), once; best effort: without the right, they stay normal.
    fn match_priority(&mut self) {
        if std::mem::replace(&mut self.prioritized, true) {
            return;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::thread::JoinHandleExt;
            let mut policy = 0;
            let mut param: libc::sched_param = std::mem::zeroed();
            if libc::pthread_getschedparam(libc::pthread_self(), &mut policy, &mut param) == 0 && policy != libc::SCHED_OTHER {
                for h in &self.handles {
                    libc::pthread_setschedparam(h.as_pthread_t(), policy, &param);
                }
            }
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        for h in self.handles.drain(..) {
            h.thread().unpark();
            let _ = h.join();
        }
    }
}

fn worker(sh: Arc<Shared>, id: usize) {
    // Plugins ask the host which thread calls them: this one renders.
    crate::host::mark_audio_thread();
    let mut seen = 0u32;
    loop {
        if sh.quit.load(Ordering::Acquire) {
            return;
        }
        let block = (sh.state.load(Ordering::Acquire) >> 32) as u32;
        if block == seen {
            // An unpark between the load and here makes this return at once.
            std::thread::park();
            continue;
        }
        seen = block;
        sh.drain(block, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn add(ctx: *const (), job: usize, _worker: usize) {
        let v = &*(ctx as *const Vec<AtomicU32>);
        v[job].fetch_add(job as u32 + 1, Ordering::Relaxed);
    }

    #[test]
    fn every_job_runs_once_per_block() {
        let mut w = Workers::new(3);
        let v: Vec<AtomicU32> = (0..40).map(|_| AtomicU32::new(0)).collect();
        for _ in 0..1000 {
            unsafe { w.run(v.len(), add, &v as *const _ as *const ()) };
        }
        for (j, x) in v.iter().enumerate() {
            assert_eq!(x.load(Ordering::Relaxed), 1000 * (j as u32 + 1));
        }
    }
}
