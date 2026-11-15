//! Marker metadata and flight plans, independent of Vulkan.

/// A named command-stream marker with a one-based identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// Marker id stored in the GPU-visible slot; zero is reserved as unwritten.
    pub id: u32,
    /// Human-readable command-buffer position.
    pub label: String,
}

/// Ordered markers to emit in a command stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerPlan {
    markers: Vec<Marker>,
}

impl MarkerPlan {
    /// Creates a plan from ordered labels, assigning ids starting at one.
    pub fn new(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let markers = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| Marker {
                id: u32::try_from(index + 1).expect("marker plan exceeds u32 ids"),
                label: label.into(),
            })
            .collect();
        Self { markers }
    }

    /// Markers around each dispatch in a workload.
    pub fn around_dispatches(count: usize) -> Self {
        Self::new((0..count).flat_map(|index| {
            [
                format!("dispatch {index} begin"),
                format!("dispatch {index} complete"),
            ]
        }))
    }

    /// Number of planned markers.
    pub fn len(&self) -> usize {
        self.markers.len()
    }

    /// Whether the plan contains no markers.
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }

    /// Marker metadata for a one-based id.
    pub fn get(&self, id: u32) -> Option<&Marker> {
        id.checked_sub(1)
            .and_then(|index| self.markers.get(index as usize))
    }

    /// Ordered marker metadata.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }
}

/// Decodes a marker word; zero means that the GPU has not written that slot.
pub fn decode(word: u32) -> Option<u32> {
    (word != 0).then_some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_assigns_ordered_ids_and_looks_up_labels() {
        let plan = MarkerPlan::new(["begin", "dispatch", "end"]);
        assert_eq!(plan.len(), 3);
        assert!(!plan.is_empty());
        assert_eq!(
            plan.get(1).map(|marker| marker.label.as_str()),
            Some("begin")
        );
        assert_eq!(plan.get(3).map(|marker| marker.id), Some(3));
        assert_eq!(plan.get(0), None);
        assert_eq!(plan.get(4), None);
        assert_eq!(plan.markers()[1].label, "dispatch");
    }

    #[test]
    fn empty_plan_and_dispatch_plan_have_expected_shapes() {
        let empty = MarkerPlan::new(Vec::<String>::new());
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        let plan = MarkerPlan::around_dispatches(2);
        assert_eq!(plan.len(), 4);
        assert_eq!(
            plan.get(1).map(|m| m.label.as_str()),
            Some("dispatch 0 begin")
        );
        assert_eq!(
            plan.get(4).map(|m| m.label.as_str()),
            Some("dispatch 1 complete")
        );
        assert!(MarkerPlan::around_dispatches(0).is_empty());
    }

    #[test]
    fn zero_word_is_unwritten_and_nonzero_words_are_ids() {
        assert_eq!(decode(0), None);
        assert_eq!(decode(1), Some(1));
        assert_eq!(decode(u32::MAX), Some(u32::MAX));
    }
}
