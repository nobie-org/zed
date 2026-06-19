//! Retained-layout work and miss accounting.
//!
//! The forest records typed events. This module owns how those events become
//! frame telemetry, miss summaries, trace sample indexes, and test mutation
//! samples.

/// Counts retained-forest operations performed during a frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::layout) struct RetainedLayoutWork {
    pub(in crate::layout) creates: u64,
    pub(in crate::layout) reuses: u64,
    pub(in crate::layout) style_updates: u64,
    pub(in crate::layout) child_list_updates: u64,
    pub(in crate::layout) measured_context_clears: u64,
    pub(in crate::layout) removes: u64,
    pub(in crate::layout) snapshot_hits: u64,
    pub(in crate::layout) snapshot_misses: u64,
    pub(in crate::layout) snapshot_text_artifact_replays: u64,
}

/// Counts why a retained occurrence could not be reused.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::layout) struct RetainedLayoutMissWork {
    pub(in crate::layout) no_previous: u64,
    pub(in crate::layout) style: u64,
    pub(in crate::layout) kind: u64,
    pub(in crate::layout) measured_kind: u64,
    pub(in crate::layout) child_count: u64,
    pub(in crate::layout) child_subtree: u64,
    pub(in crate::layout) no_exact_child: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::layout) struct RetainedForestMutationSample {
    pub(in crate::layout) creates: u64,
    pub(in crate::layout) reuses: u64,
    pub(in crate::layout) style_updates: u64,
    pub(in crate::layout) child_list_updates: u64,
    pub(in crate::layout) context_clears: u64,
    pub(in crate::layout) removes: u64,
    pub(in crate::layout) snapshot_hits: u64,
    pub(in crate::layout) snapshot_misses: u64,
    pub(in crate::layout) snapshot_text_artifact_replays: u64,
}

/// Mutable retained-layout accounting for the current frame.
pub(super) struct RetainedWorkState {
    work: RetainedLayoutWork,
    miss_work: RetainedLayoutMissWork,
    miss_trace_samples: usize,
    #[cfg(test)]
    mutation_sample_for_tests: RetainedForestMutationSample,
}

/// Transaction checkpoint for retained work accounting.
pub(super) struct RetainedWorkCheckpoint {
    work: RetainedLayoutWork,
    miss_work: RetainedLayoutMissWork,
    miss_trace_samples: usize,
    #[cfg(test)]
    mutation_sample_for_tests: RetainedForestMutationSample,
}

/// Delta of retained work performed by one forest operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RetainedWorkDelta {
    pub(super) work: RetainedLayoutWork,
    pub(super) miss_work: RetainedLayoutMissWork,
}

impl RetainedWorkDelta {
    pub(super) fn miss_count(&self) -> u64 {
        self.miss_work.no_previous
            + self.miss_work.style
            + self.miss_work.kind
            + self.miss_work.measured_kind
            + self.miss_work.child_count
            + self.miss_work.child_subtree
            + self.miss_work.no_exact_child
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RetainedWorkSnapshot {
    work: RetainedLayoutWork,
    miss_work: RetainedLayoutMissWork,
}

impl RetainedWorkState {
    pub(super) fn new() -> Self {
        Self {
            work: RetainedLayoutWork::default(),
            miss_work: RetainedLayoutMissWork::default(),
            miss_trace_samples: 0,
            #[cfg(test)]
            mutation_sample_for_tests: RetainedForestMutationSample::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.work = RetainedLayoutWork::default();
        self.miss_work = RetainedLayoutMissWork::default();
    }

    pub(super) fn checkpoint(&self) -> RetainedWorkCheckpoint {
        RetainedWorkCheckpoint {
            work: self.work,
            miss_work: self.miss_work,
            miss_trace_samples: self.miss_trace_samples,
            #[cfg(test)]
            mutation_sample_for_tests: self.mutation_sample_for_tests,
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RetainedWorkCheckpoint) {
        self.work = checkpoint.work;
        self.miss_work = checkpoint.miss_work;
        self.miss_trace_samples = checkpoint.miss_trace_samples;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests = checkpoint.mutation_sample_for_tests;
        }
    }

    pub(super) fn miss_work(&self) -> RetainedLayoutMissWork {
        self.miss_work
    }

    pub(super) fn snapshot(&self) -> RetainedWorkSnapshot {
        RetainedWorkSnapshot {
            work: self.work,
            miss_work: self.miss_work,
        }
    }

    pub(super) fn delta_since(&self, snapshot: RetainedWorkSnapshot) -> RetainedWorkDelta {
        RetainedWorkDelta {
            work: RetainedLayoutWork {
                creates: self.work.creates - snapshot.work.creates,
                reuses: self.work.reuses - snapshot.work.reuses,
                style_updates: self.work.style_updates - snapshot.work.style_updates,
                child_list_updates: self.work.child_list_updates - snapshot.work.child_list_updates,
                measured_context_clears: self.work.measured_context_clears
                    - snapshot.work.measured_context_clears,
                removes: self.work.removes - snapshot.work.removes,
                snapshot_hits: self.work.snapshot_hits - snapshot.work.snapshot_hits,
                snapshot_misses: self.work.snapshot_misses - snapshot.work.snapshot_misses,
                snapshot_text_artifact_replays: self.work.snapshot_text_artifact_replays
                    - snapshot.work.snapshot_text_artifact_replays,
            },
            miss_work: RetainedLayoutMissWork {
                no_previous: self.miss_work.no_previous - snapshot.miss_work.no_previous,
                style: self.miss_work.style - snapshot.miss_work.style,
                kind: self.miss_work.kind - snapshot.miss_work.kind,
                measured_kind: self.miss_work.measured_kind - snapshot.miss_work.measured_kind,
                child_count: self.miss_work.child_count - snapshot.miss_work.child_count,
                child_subtree: self.miss_work.child_subtree - snapshot.miss_work.child_subtree,
                no_exact_child: self.miss_work.no_exact_child - snapshot.miss_work.no_exact_child,
            },
        }
    }

    pub(super) fn finish_frame(&mut self) -> (RetainedLayoutWork, RetainedLayoutMissWork) {
        let work = self.work;
        let misses = self.miss_work;
        self.work = RetainedLayoutWork::default();
        self.miss_work = RetainedLayoutMissWork::default();
        (work, misses)
    }

    pub(super) fn should_trace_miss(&self, limit: usize) -> bool {
        self.miss_trace_samples < limit
    }

    pub(super) fn take_miss_trace_sample_index(&mut self) -> usize {
        let index = self.miss_trace_samples;
        self.miss_trace_samples += 1;
        index
    }

    pub(super) fn record_create(&mut self) {
        self.work.creates += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.creates += 1;
        }
    }

    pub(super) fn record_reuse(&mut self) {
        self.work.reuses += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.reuses += 1;
        }
    }

    pub(super) fn record_style_update(&mut self) {
        self.work.style_updates += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.style_updates += 1;
        }
    }

    pub(super) fn record_child_list_update(&mut self) {
        self.work.child_list_updates += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.child_list_updates += 1;
        }
    }

    pub(super) fn record_measured_context_clear(&mut self) {
        self.work.measured_context_clears += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.context_clears += 1;
        }
    }

    pub(super) fn record_remove(&mut self) {
        self.work.removes += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.removes += 1;
        }
    }

    pub(super) fn record_snapshot_hit(&mut self, text_artifact_replays: u64) {
        self.work.snapshot_hits += 1;
        self.work.snapshot_text_artifact_replays += text_artifact_replays;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.snapshot_hits += 1;
            self.mutation_sample_for_tests
                .snapshot_text_artifact_replays += text_artifact_replays;
        }
    }

    pub(super) fn record_snapshot_miss(&mut self) {
        self.work.snapshot_misses += 1;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests.snapshot_misses += 1;
        }
    }

    pub(super) fn record_snapshot_text_artifact_replays(&mut self, text_artifact_replays: u64) {
        self.work.snapshot_text_artifact_replays += text_artifact_replays;
        #[cfg(test)]
        {
            self.mutation_sample_for_tests
                .snapshot_text_artifact_replays += text_artifact_replays;
        }
    }

    pub(super) fn record_no_previous_miss(&mut self) {
        self.miss_work.no_previous += 1;
    }

    pub(super) fn record_measured_kind_miss(&mut self) {
        self.miss_work.measured_kind += 1;
    }

    #[cfg(test)]
    pub(super) fn reset_mutation_sample_for_tests(&mut self) {
        self.mutation_sample_for_tests = RetainedForestMutationSample::default();
    }

    #[cfg(test)]
    pub(super) fn mutation_sample_for_tests(&self) -> RetainedForestMutationSample {
        self.mutation_sample_for_tests
    }
}
