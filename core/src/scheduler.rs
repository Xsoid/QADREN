//! Deterministic automatic-schedule evaluation.
//!
//! This module has no clock, SQLite or Job submission. Callers supply the
//! observed UTC interval and persist the returned occurrence key before they
//! create a Job. `chrono-tz` is used only for the IANA timezone database; it
//! introduces no end-user runtime service or network dependency.
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklySchedule {
    pub timezone: Tz,
    pub weekday: Weekday,
    pub hour: u32,
    pub minute: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    InvalidTime,
    InvalidInterval,
}

/// A stable local-slot identity. The repeated DST hour maps to one key, so
/// callers can persist it and never create two Jobs for the same slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OccurrenceKey(pub String);

impl WeeklySchedule {
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if self.hour > 23 || self.minute > 59 {
            Err(ScheduleError::InvalidTime)
        } else {
            Ok(())
        }
    }

    /// Return the local schedule slots whose UTC observations fall in
    /// `(after, through]`. Nonexistent DST slots yield no occurrence; repeated
    /// slots yield one shared key even though two UTC instants exist.
    pub fn occurrences_between(
        &self,
        after: DateTime<Utc>,
        through: DateTime<Utc>,
    ) -> Result<Vec<OccurrenceKey>, ScheduleError> {
        self.validate()?;
        if through < after {
            return Err(ScheduleError::InvalidInterval);
        }
        let start = after.with_timezone(&self.timezone).date_naive() - Duration::days(1);
        let end = through.with_timezone(&self.timezone).date_naive() + Duration::days(1);
        let mut keys = Vec::new();
        let mut date = start;
        while date <= end {
            if date.weekday() == self.weekday {
                let local = date
                    .and_hms_opt(self.hour, self.minute, 0)
                    .expect("validated time");
                match self.timezone.from_local_datetime(&local) {
                    chrono::LocalResult::Single(instant) => {
                        if instant.with_timezone(&Utc) > after
                            && instant.with_timezone(&Utc) <= through
                        {
                            keys.push(OccurrenceKey(format!(
                                "{}@{}",
                                local.format("%F %R"),
                                self.timezone.name()
                            )));
                        }
                    }
                    chrono::LocalResult::Ambiguous(first, second) => {
                        if first.with_timezone(&Utc) > after && first.with_timezone(&Utc) <= through
                            || second.with_timezone(&Utc) > after
                                && second.with_timezone(&Utc) <= through
                        {
                            keys.push(OccurrenceKey(format!(
                                "{}@{}",
                                local.format("%F %R"),
                                self.timezone.name()
                            )));
                        }
                    }
                    chrono::LocalResult::None => {}
                }
            }
            date = date.succ_opt().expect("bounded date range");
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    #[test]
    fn repeated_dst_hour_produces_one_persistable_slot() {
        let schedule = WeeklySchedule {
            timezone: chrono_tz::Europe::Berlin,
            weekday: Weekday::Sun,
            hour: 2,
            minute: 30,
        };
        let slots = schedule
            .occurrences_between(utc("2026-10-25T00:00:00Z"), utc("2026-10-25T02:00:00Z"))
            .unwrap();
        assert_eq!(
            slots,
            vec![OccurrenceKey("2026-10-25 02:30@Europe/Berlin".into())]
        );
    }

    #[test]
    fn interval_excludes_its_start_and_includes_its_end() {
        let schedule = WeeklySchedule {
            timezone: chrono_tz::UTC,
            weekday: Weekday::Mon,
            hour: 9,
            minute: 0,
        };
        let slot = utc("2026-10-19T09:00:00Z");
        assert_eq!(
            schedule
                .occurrences_between(slot - Duration::minutes(1), slot)
                .unwrap()
                .len(),
            1
        );
        assert!(schedule
            .occurrences_between(slot, slot + Duration::minutes(1))
            .unwrap()
            .is_empty());
        assert_eq!(
            schedule.occurrences_between(slot, slot - Duration::minutes(1)),
            Err(ScheduleError::InvalidInterval)
        );
    }

    #[test]
    fn new_york_dst_transitions_keep_one_slot_or_none() {
        let schedule = WeeklySchedule {
            timezone: chrono_tz::America::New_York,
            weekday: Weekday::Sun,
            hour: 1,
            minute: 30,
        };
        // 2026-11-01 01:30 local occurs twice. One persistable key exists.
        assert_eq!(
            schedule
                .occurrences_between(utc("2026-11-01T04:00:00Z"), utc("2026-11-01T07:00:00Z"))
                .unwrap(),
            vec![OccurrenceKey("2026-11-01 01:30@America/New_York".into())]
        );
        let spring = WeeklySchedule {
            hour: 2,
            ..schedule
        };
        // 2026-03-08 02:30 local does not exist.
        assert!(spring
            .occurrences_between(utc("2026-03-08T05:00:00Z"), utc("2026-03-08T09:00:00Z"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn long_gap_yields_each_weekly_slot_once_across_a_dst_change() {
        let schedule = WeeklySchedule {
            timezone: chrono_tz::Europe::Berlin,
            weekday: Weekday::Mon,
            hour: 9,
            minute: 0,
        };
        let slots = schedule
            .occurrences_between(utc("2026-10-12T00:00:00Z"), utc("2026-11-09T23:59:00Z"))
            .unwrap();
        assert_eq!(
            slots,
            vec![
                OccurrenceKey("2026-10-12 09:00@Europe/Berlin".into()),
                OccurrenceKey("2026-10-19 09:00@Europe/Berlin".into()),
                OccurrenceKey("2026-10-26 09:00@Europe/Berlin".into()),
                OccurrenceKey("2026-11-02 09:00@Europe/Berlin".into()),
                OccurrenceKey("2026-11-09 09:00@Europe/Berlin".into()),
            ]
        );
    }

    #[test]
    fn missing_dst_hour_never_creates_a_slot() {
        let schedule = WeeklySchedule {
            timezone: chrono_tz::Europe::Berlin,
            weekday: Weekday::Sun,
            hour: 2,
            minute: 30,
        };
        let slots = schedule
            .occurrences_between(utc("2026-03-29T00:00:00Z"), utc("2026-03-29T03:00:00Z"))
            .unwrap();
        assert!(slots.is_empty());
    }
}
