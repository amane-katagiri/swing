use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
struct Cursor {
    value: AtomicU64,
    checked: AtomicBool,
}

impl Cursor {
    fn record(&self, at: u64) {
        self.value.fetch_max(at, Ordering::Relaxed);
    }

    fn mark_checked(&self) {
        self.checked.store(true, Ordering::Relaxed);
    }

    fn latest(&self) -> Option<u64> {
        let value = self.value.load(Ordering::Relaxed);
        (value != 0 || self.checked.load(Ordering::Relaxed)).then_some(value)
    }
}

#[derive(Default)]
pub struct Activity {
    published: Cursor,
    replica_reports: Cursor,
}

impl Activity {
    pub fn record_published(&self, created_at: u64) {
        self.published.record(created_at);
    }

    pub fn mark_published_checked(&self) {
        self.published.mark_checked();
    }

    pub fn latest_published_at(&self) -> Option<u64> {
        self.published.latest()
    }

    pub fn record_replica_report(&self, created_at: u64) {
        self.replica_reports.record(created_at);
    }

    pub fn mark_replica_reports_checked(&self) {
        self.replica_reports.mark_checked();
    }

    pub fn latest_replica_report_at(&self) -> Option<u64> {
        self.replica_reports.latest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_start_unknown_and_never_go_down() {
        let activity = Activity::default();
        assert_eq!(activity.latest_published_at(), None);
        assert_eq!(activity.latest_replica_report_at(), None);

        activity.record_published(200);
        activity.record_published(100);
        activity.record_replica_report(300);
        activity.record_replica_report(250);

        assert_eq!(activity.latest_published_at(), Some(200));
        assert_eq!(activity.latest_replica_report_at(), Some(300));
    }

    #[test]
    fn a_check_that_found_nothing_reads_as_zero() {
        let activity = Activity::default();
        activity.mark_published_checked();
        activity.mark_replica_reports_checked();
        assert_eq!(activity.latest_published_at(), Some(0));
        assert_eq!(activity.latest_replica_report_at(), Some(0));

        activity.record_replica_report(300);
        assert_eq!(activity.latest_replica_report_at(), Some(300));
    }
}
