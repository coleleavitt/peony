use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ws_deque::{Steal, Worker};

use super::{GcContext, GcWorkItem};

pub(super) struct ParallelLevel {
    pub(super) candidates: Vec<(usize, usize)>,
    pub(super) traversed_sections: u64,
    pub(super) scanned_relocs: u64,
}

pub(super) fn collect_parallel_level(
    ctx: GcContext<'_, '_>,
    frontier: &mut Vec<GcWorkItem>,
    pl: usize,
) -> ParallelLevel {
    let workers: Vec<Worker<GcWorkItem>> = (0..pl).map(|_| Worker::new()).collect();
    let stealers: Vec<_> = workers.iter().map(|w| w.stealer()).collect();
    for (index, item) in frontier.drain(..).enumerate() {
        workers[index % pl].push(item);
    }
    let results = Arc::new(Mutex::new(Vec::new()));
    let idle_count = Arc::new(AtomicUsize::new(0));
    let traversed_sections = Arc::new(AtomicU64::new(0));
    let scanned_relocs = Arc::new(AtomicU64::new(0));

    std::thread::scope(|scope| {
        for (thread_index, worker) in workers.into_iter().enumerate() {
            let all_stealers: Vec<_> = stealers.to_vec();
            let results = Arc::clone(&results);
            let idle_count = Arc::clone(&idle_count);
            let traversed_sections = Arc::clone(&traversed_sections);
            let scanned_relocs = Arc::clone(&scanned_relocs);
            scope.spawn(move || {
                let mut local_out = Vec::new();
                let mut local_sections = 0u64;
                let mut local_relocs = 0u64;
                let mut is_idle = false;
                loop {
                    if let Some(item) = worker.pop() {
                        if is_idle {
                            idle_count.fetch_sub(1, Ordering::Release);
                            is_idle = false;
                        }
                        local_sections += 1;
                        local_relocs += ctx.collect_targets(item, &mut local_out) as u64;
                        continue;
                    }
                    let mut found = false;
                    for (index, stealer) in all_stealers.iter().enumerate() {
                        if index == thread_index {
                            continue;
                        }
                        match stealer.steal() {
                            Steal::Success(item) => {
                                if is_idle {
                                    idle_count.fetch_sub(1, Ordering::Release);
                                    is_idle = false;
                                }
                                local_sections += 1;
                                local_relocs += ctx.collect_targets(item, &mut local_out) as u64;
                                found = true;
                                break;
                            }
                            Steal::Retry => {
                                found = true;
                                break;
                            }
                            Steal::Empty => {}
                        }
                    }
                    if found {
                        continue;
                    }
                    if !is_idle {
                        idle_count.fetch_add(1, Ordering::Release);
                        is_idle = true;
                    }
                    if idle_count.load(Ordering::Acquire) >= pl {
                        break;
                    }
                    std::hint::spin_loop();
                }
                if !local_out.is_empty() {
                    extend_locked(&results, local_out);
                }
                traversed_sections.fetch_add(local_sections, Ordering::Relaxed);
                scanned_relocs.fetch_add(local_relocs, Ordering::Relaxed);
            });
        }
    });

    ParallelLevel {
        candidates: take_results(results),
        traversed_sections: traversed_sections.load(Ordering::Relaxed),
        scanned_relocs: scanned_relocs.load(Ordering::Relaxed),
    }
}

fn extend_locked<T>(results: &Mutex<Vec<T>>, values: Vec<T>) {
    match results.lock() {
        Ok(mut guard) => guard.extend(values),
        Err(poisoned) => poisoned.into_inner().extend(values),
    }
}

fn take_results<T>(results: Arc<Mutex<Vec<T>>>) -> Vec<T> {
    match Arc::try_unwrap(results) {
        Ok(mutex) => mutex
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        Err(shared) => match shared.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(poisoned) => std::mem::take(poisoned.into_inner().as_mut()),
        },
    }
}
