//! Deterministic automatic-schedule evaluation.
//!
//! This module has no clock, SQLite or Job submission. Callers supply the
//! observed UTC interval and persist the returned occurrence key before they
//! create a Job. `chrono-tz` is used only for the IANA timezone database; it
//! introduces no end-user runtime service or network dependency.
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

/// Monday is bit 0 and Sunday is bit 6. Daily is all seven bits.
pub const ALL_DAYS: u8 = 0x7f;

/// The rule of one schedule (D-025, issue #34).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleRule {
    /// Selected local days at one local time. A missing local time (DST gap)
    /// gives no slot; a repeated local time gives one slot.
    DaysAtTime { hour: u32, minute: u32 },
    /// Every N hours from a fixed UTC anchor. Slots follow UTC, so a DST
    /// change neither shifts nor doubles them.
    EveryHours {
        interval_hours: u32,
        anchor: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub timezone: Tz,
    /// Active local days. A slot on another local day is not a slot.
    pub days_mask: u8,
    pub rule: ScheduleRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    InvalidTime,
    InvalidDays,
    InvalidInterval,
}

/// A stable slot identity. The repeated DST hour maps to one key, so
/// callers can persist it and never create two Jobs for the same slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OccurrenceKey(pub String);

/// One slot: its durable key and the first UTC instant of it in the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub key: OccurrenceKey,
    pub at: DateTime<Utc>,
}

impl Schedule {
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if self.days_mask == 0 || self.days_mask > ALL_DAYS {
            return Err(ScheduleError::InvalidDays);
        }
        match self.rule {
            ScheduleRule::DaysAtTime { hour, minute } if hour > 23 || minute > 59 => {
                Err(ScheduleError::InvalidTime)
            }
            ScheduleRule::EveryHours { interval_hours, .. }
                if !(1..=24).contains(&interval_hours) =>
            {
                Err(ScheduleError::InvalidInterval)
            }
            _ => Ok(()),
        }
    }

    fn active_on(&self, date: NaiveDate) -> bool {
        self.days_mask & (1 << date.weekday().num_days_from_monday()) != 0
    }

    /// Return the slots whose UTC instants fall in `(after, through]`.
    pub fn occurrences_between(
        &self,
        after: DateTime<Utc>,
        through: DateTime<Utc>,
    ) -> Result<Vec<Occurrence>, ScheduleError> {
        self.validate()?;
        if through < after {
            return Err(ScheduleError::InvalidInterval);
        }
        let inside = |instant: DateTime<Utc>| instant > after && instant <= through;
        let mut slots = Vec::new();
        match self.rule {
            ScheduleRule::DaysAtTime { hour, minute } => {
                let mut date = after.with_timezone(&self.timezone).date_naive() - Duration::days(1);
                let end = through.with_timezone(&self.timezone).date_naive() + Duration::days(1);
                while date <= end {
                    if self.active_on(date) {
                        let local = date.and_hms_opt(hour, minute, 0).expect("validated time");
                        let first = match self.timezone.from_local_datetime(&local) {
                            chrono::LocalResult::Single(instant) => {
                                Some(instant.with_timezone(&Utc)).filter(|at| inside(*at))
                            }
                            chrono::LocalResult::Ambiguous(first, second) => [first, second]
                                .into_iter()
                                .map(|instant| instant.with_timezone(&Utc))
                                .find(|at| inside(*at)),
                            chrono::LocalResult::None => None,
                        };
                        if let Some(at) = first {
                            slots.push(Occurrence {
                                key: OccurrenceKey(format!(
                                    "{}@{}",
                                    local.format("%F %R"),
                                    self.timezone.name()
                                )),
                                at,
                            });
                        }
                    }
                    date = date.succ_opt().expect("bounded date range");
                }
            }
            ScheduleRule::EveryHours {
                interval_hours,
                anchor,
            } => {
                let step = Duration::hours(interval_hours.into());
                let steps = (after - anchor)
                    .num_seconds()
                    .div_euclid(step.num_seconds())
                    + 1;
                let mut at = anchor + step * steps as i32;
                while at <= through {
                    if self.active_on(at.with_timezone(&self.timezone).date_naive()) {
                        slots.push(Occurrence {
                            key: OccurrenceKey(format!(
                                "every-{interval_hours}h@{}",
                                at.format("%Y-%m-%dT%H:%MZ")
                            )),
                            at,
                        });
                    }
                    at += step;
                }
            }
        }
        Ok(slots)
    }

    /// The first slot after `after`. Every valid schedule has one within
    /// eight days.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<Option<Occurrence>, ScheduleError> {
        Ok(self
            .occurrences_between(after, after + Duration::days(8))?
            .into_iter()
            .next())
    }
}

/// The UTC anchor of an every-N-hours schedule: the first instant of the
/// local time `hour:minute` on `date` in `timezone`. A local time in a DST
/// gap moves forward by one hour.
pub fn every_hours_anchor(
    timezone: Tz,
    date: NaiveDate,
    hour: u32,
    minute: u32,
) -> Result<DateTime<Utc>, ScheduleError> {
    let local = date
        .and_hms_opt(hour, minute, 0)
        .ok_or(ScheduleError::InvalidTime)?;
    timezone
        .from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            timezone
                .from_local_datetime(&(local + Duration::hours(1)))
                .earliest()
        })
        .map(|instant| instant.with_timezone(&Utc))
        .ok_or(ScheduleError::InvalidTime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    fn weekly(timezone: Tz, weekday: Weekday, hour: u32, minute: u32) -> Schedule {
        Schedule {
            timezone,
            days_mask: 1 << weekday.num_days_from_monday(),
            rule: ScheduleRule::DaysAtTime { hour, minute },
        }
    }

    fn keys(schedule: &Schedule, after: &str, through: &str) -> Vec<String> {
        schedule
            .occurrences_between(utc(after), utc(through))
            .unwrap()
            .into_iter()
            .map(|slot| slot.key.0)
            .collect()
    }

    fn instants(schedule: &Schedule, after: &str, through: &str) -> Vec<String> {
        schedule
            .occurrences_between(utc(after), utc(through))
            .unwrap()
            .into_iter()
            .map(|slot| slot.at.format("%m-%dT%H:%M").to_string())
            .collect()
    }

    #[test]
    fn repeated_dst_hour_produces_one_persistable_slot() {
        let schedule = weekly(chrono_tz::Europe::Berlin, Weekday::Sun, 2, 30);
        assert_eq!(
            keys(&schedule, "2026-10-25T00:00:00Z", "2026-10-25T02:00:00Z"),
            ["2026-10-25 02:30@Europe/Berlin"]
        );
    }

    #[test]
    fn interval_excludes_its_start_and_includes_its_end() {
        let schedule = weekly(chrono_tz::UTC, Weekday::Mon, 9, 0);
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
        let schedule = weekly(chrono_tz::America::New_York, Weekday::Sun, 1, 30);
        // 2026-11-01 01:30 local occurs twice. One persistable key exists.
        assert_eq!(
            keys(&schedule, "2026-11-01T04:00:00Z", "2026-11-01T07:00:00Z"),
            ["2026-11-01 01:30@America/New_York"]
        );
        let spring = weekly(chrono_tz::America::New_York, Weekday::Sun, 2, 30);
        // 2026-03-08 02:30 local does not exist.
        assert!(spring
            .occurrences_between(utc("2026-03-08T05:00:00Z"), utc("2026-03-08T09:00:00Z"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn long_gap_yields_each_weekly_slot_once_across_a_dst_change() {
        let schedule = weekly(chrono_tz::Europe::Berlin, Weekday::Mon, 9, 0);
        assert_eq!(
            keys(&schedule, "2026-10-12T00:00:00Z", "2026-11-09T23:59:00Z"),
            [
                "2026-10-12 09:00@Europe/Berlin",
                "2026-10-19 09:00@Europe/Berlin",
                "2026-10-26 09:00@Europe/Berlin",
                "2026-11-02 09:00@Europe/Berlin",
                "2026-11-09 09:00@Europe/Berlin",
            ]
        );
    }

    #[test]
    fn missing_dst_hour_never_creates_a_slot() {
        let schedule = weekly(chrono_tz::Europe::Berlin, Weekday::Sun, 2, 30);
        let slots = schedule
            .occurrences_between(utc("2026-03-29T00:00:00Z"), utc("2026-03-29T03:00:00Z"))
            .unwrap();
        assert!(slots.is_empty());
    }

    #[test]
    fn daily_keeps_the_local_time_across_dst_changes() {
        let berlin = Schedule {
            timezone: chrono_tz::Europe::Berlin,
            days_mask: ALL_DAYS,
            rule: ScheduleRule::DaysAtTime { hour: 3, minute: 0 },
        };
        // Summer time ends on 2026-10-25: 03:00 local moves from 01:00Z to 02:00Z.
        assert_eq!(
            instants(&berlin, "2026-10-23T12:00:00Z", "2026-10-26T12:00:00Z"),
            ["10-24T01:00", "10-25T02:00", "10-26T02:00"]
        );
        // The 02:30 slot on 2026-03-29 does not exist; the other days do.
        let gap = Schedule {
            rule: ScheduleRule::DaysAtTime {
                hour: 2,
                minute: 30,
            },
            ..berlin.clone()
        };
        assert_eq!(
            keys(&gap, "2026-03-27T12:00:00Z", "2026-03-30T12:00:00Z"),
            [
                "2026-03-28 02:30@Europe/Berlin",
                "2026-03-30 02:30@Europe/Berlin"
            ]
        );
        let new_york = Schedule {
            timezone: chrono_tz::America::New_York,
            days_mask: ALL_DAYS,
            rule: ScheduleRule::DaysAtTime {
                hour: 1,
                minute: 30,
            },
        };
        // 2026-11-01 01:30 local occurs twice: one slot at the first instant.
        assert_eq!(
            instants(&new_york, "2026-10-31T12:00:00Z", "2026-11-02T12:00:00Z"),
            ["11-01T05:30", "11-02T06:30"]
        );
    }

    #[test]
    fn selected_days_skip_the_other_days() {
        let weekdays = Schedule {
            timezone: chrono_tz::Europe::Moscow,
            days_mask: 0b001_1111,
            rule: ScheduleRule::DaysAtTime { hour: 3, minute: 0 },
        };
        // 2026-10-30 is a Friday; Saturday and Sunday give no slot.
        assert_eq!(
            keys(&weekdays, "2026-10-29T12:00:00Z", "2026-11-02T12:00:00Z"),
            [
                "2026-10-30 03:00@Europe/Moscow",
                "2026-11-02 03:00@Europe/Moscow"
            ]
        );
        for mask in [0, 0x80] {
            assert_eq!(
                Schedule {
                    days_mask: mask,
                    ..weekdays.clone()
                }
                .validate(),
                Err(ScheduleError::InvalidDays)
            );
        }
    }

    #[test]
    fn every_hours_follows_utc_across_dst_changes() {
        let anchor = every_hours_anchor(
            chrono_tz::Europe::Berlin,
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            0,
            0,
        )
        .unwrap();
        assert_eq!(anchor, utc("2026-09-30T22:00:00Z"));
        let every = |interval_hours| Schedule {
            timezone: chrono_tz::Europe::Berlin,
            days_mask: ALL_DAYS,
            rule: ScheduleRule::EveryHours {
                interval_hours,
                anchor,
            },
        };
        // The repeated hour on 2026-10-25 gives each UTC hour once.
        assert_eq!(
            instants(&every(1), "2026-10-24T23:30:00Z", "2026-10-25T02:00:00Z"),
            ["10-25T00:00", "10-25T01:00", "10-25T02:00"]
        );
        assert_eq!(
            keys(&every(6), "2026-10-24T12:00:00Z", "2026-10-25T12:00:00Z"),
            [
                "every-6h@2026-10-24T16:00Z",
                "every-6h@2026-10-24T22:00Z",
                "every-6h@2026-10-25T04:00Z",
                "every-6h@2026-10-25T10:00Z"
            ]
        );
        // Every 24 hours keeps the UTC instant, so local time moves one hour.
        assert_eq!(
            instants(&every(24), "2026-10-23T12:00:00Z", "2026-10-26T12:00:00Z"),
            ["10-23T22:00", "10-24T22:00", "10-25T22:00"]
        );
        let new_york = Schedule {
            timezone: chrono_tz::America::New_York,
            days_mask: ALL_DAYS,
            rule: ScheduleRule::EveryHours {
                interval_hours: 6,
                anchor: utc("2026-03-01T05:00:00Z"),
            },
        };
        assert_eq!(
            instants(&new_york, "2026-03-07T23:00:00Z", "2026-03-08T12:00:00Z"),
            ["03-08T05:00", "03-08T11:00"]
        );
        for interval_hours in [0, 25] {
            assert_eq!(
                every(interval_hours).validate(),
                Err(ScheduleError::InvalidInterval)
            );
        }
    }

    #[test]
    fn every_hours_respects_active_days_and_reports_the_next_slot() {
        let schedule = Schedule {
            timezone: chrono_tz::UTC,
            days_mask: 1 << Weekday::Mon.num_days_from_monday(),
            rule: ScheduleRule::EveryHours {
                interval_hours: 12,
                anchor: utc("2026-10-01T00:00:00Z"),
            },
        };
        // 2026-10-30 is a Friday; the next active slot is Monday 00:00.
        let next = schedule
            .next_after(utc("2026-10-30T13:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(next.at, utc("2026-11-02T00:00:00Z"));
        assert_eq!(next.key.0, "every-12h@2026-11-02T00:00Z");
    }

    #[test]
    fn anchor_in_a_dst_gap_moves_forward() {
        let anchor = every_hours_anchor(
            chrono_tz::Europe::Berlin,
            NaiveDate::from_ymd_opt(2026, 3, 29).unwrap(),
            2,
            30,
        )
        .unwrap();
        assert_eq!(anchor, utc("2026-03-29T01:30:00Z"));
    }
}
