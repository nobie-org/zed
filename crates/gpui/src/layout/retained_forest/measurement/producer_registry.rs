use super::LayoutMeasureContext;

/// Current-frame executable producers for measured layout nodes.
///
/// These callbacks are frame-local effects. They are not retained layout facts,
/// so the registry only owns producer slots and rollback length; comparable
/// measurement identity stays in `MeasuredLayoutFacts`.
pub(super) struct ProducerRegistry {
    contexts: Vec<Option<LayoutMeasureContext>>,
}

/// Transaction checkpoint for current-frame producer slots.
pub(super) struct ProducerRegistryCheckpoint {
    contexts_len: usize,
}

impl ProducerRegistry {
    pub(super) fn new() -> Self {
        Self {
            contexts: Vec::new(),
        }
    }

    pub(super) fn clear(&mut self) {
        self.contexts.clear();
    }

    pub(super) fn checkpoint(&self) -> ProducerRegistryCheckpoint {
        ProducerRegistryCheckpoint {
            contexts_len: self.contexts.len(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: ProducerRegistryCheckpoint) {
        self.contexts.truncate(checkpoint.contexts_len);
    }

    pub(super) fn push(&mut self, measure_context: LayoutMeasureContext) -> usize {
        let measure_id = self.contexts.len();
        self.contexts.push(Some(measure_context));
        measure_id
    }

    pub(super) fn context_mut(&mut self, measure_id: usize) -> Option<&mut LayoutMeasureContext> {
        self.contexts.get_mut(measure_id).and_then(Option::as_mut)
    }
}
