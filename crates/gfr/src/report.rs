//! Pure analysis of GPU-visible marker words.

use std::fmt;

use crate::marker::{decode, MarkerPlan};

/// Summary of how far a submitted marker plan progressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlightReport {
    /// Number of consecutive markers observed from the start of the plan.
    pub completed: usize,
    /// Number of markers in the plan.
    pub total: usize,
    /// Label of the last completed marker, if any.
    pub last_completed: Option<String>,
    /// First marker not observed, if any.
    pub first_incomplete: Option<String>,
}

impl fmt::Display for FlightReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.completed == self.total {
            return write!(f, "flight complete: all {} markers written", self.total);
        }
        match (&self.last_completed, &self.first_incomplete) {
            (Some(last), Some(next)) => write!(
                f,
                "flight partial: completed {}/{} through {last:?}; next incomplete marker: {next:?}",
                self.completed, self.total
            ),
            (None, Some(next)) => write!(
                f,
                "flight partial: no markers completed; first incomplete marker: {next:?}"
            ),
            _ => write!(f, "flight report: {} of {} markers completed", self.completed, self.total),
        }
    }
}

/// Analyzes ordered marker words (one slot per planned marker).
pub fn analyze(words: &[u32], plan: &MarkerPlan) -> FlightReport {
    let completed = (0..plan.len())
        .take_while(|&index| words.get(index).copied().and_then(decode) == Some(index as u32 + 1))
        .count();
    let last_completed = completed
        .checked_sub(1)
        .and_then(|index| plan.get(index as u32 + 1))
        .map(|marker| marker.label.clone());
    let first_incomplete = (completed < plan.len())
        .then(|| {
            plan.get(completed as u32 + 1)
                .map(|marker| marker.label.clone())
        })
        .flatten();
    FlightReport {
        completed,
        total: plan.len(),
        last_completed,
        first_incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_empty_partial_complete_and_corrupt_flights() {
        let plan = MarkerPlan::new(["begin", "draw", "end"]);
        let empty = analyze(&[], &plan);
        assert_eq!(empty.completed, 0);
        assert_eq!(empty.first_incomplete.as_deref(), Some("begin"));
        assert!(empty.to_string().contains("no markers completed"));

        let partial = analyze(&[1, 2, 0], &plan);
        assert_eq!(partial.completed, 2);
        assert_eq!(partial.last_completed.as_deref(), Some("draw"));
        assert_eq!(partial.first_incomplete.as_deref(), Some("end"));
        assert!(partial.to_string().contains("2/3"));

        let complete = analyze(&[1, 2, 3], &plan);
        assert_eq!(complete.completed, 3);
        assert_eq!(complete.first_incomplete, None);
        assert!(complete.to_string().contains("all 3 markers written"));

        let corrupt = analyze(&[1, 9, 3], &plan);
        assert_eq!(corrupt.completed, 1);
        assert_eq!(corrupt.first_incomplete.as_deref(), Some("draw"));
    }

    #[test]
    fn empty_plan_is_complete_and_short_buffers_are_unwritten() {
        let empty_plan = MarkerPlan::new(Vec::<String>::new());
        let report = analyze(&[], &empty_plan);
        assert_eq!(report.completed, 0);
        assert_eq!(report.total, 0);
        assert!(report.to_string().contains("all 0 markers"));

        let plan = MarkerPlan::new(["one", "two"]);
        let short = analyze(&[1], &plan);
        assert_eq!(short.completed, 1);
        assert_eq!(short.first_incomplete.as_deref(), Some("two"));
    }

    #[test]
    fn display_renders_every_publicly_constructible_state() {
        // The struct is publicly constructible, so `Display` must handle a
        // completed marker without a next one and no completed markers at all.
        let partial_without_next = FlightReport {
            completed: 1,
            total: 3,
            last_completed: Some("draw".to_string()),
            first_incomplete: None,
        };
        assert_eq!(
            partial_without_next.to_string(),
            "flight report: 1 of 3 markers completed"
        );

        let no_progress = FlightReport {
            completed: 0,
            total: 3,
            last_completed: None,
            first_incomplete: None,
        };
        assert_eq!(
            no_progress.to_string(),
            "flight report: 0 of 3 markers completed"
        );
    }
}
