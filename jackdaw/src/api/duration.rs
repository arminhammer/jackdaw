//! `10.millis()`, `30.secs()`, `24.hours()`: readable durations on integer literals.
use std::time::Duration;

/// Readable [`Duration`]s: `500.millis()`, `30.secs()`, `5.minutes()`, `24.hours()`, `30.days()`.
///
/// It is implemented for the integer types and in the [`prelude`](crate::prelude). A negative number gives a zero
/// duration.
///
/// ```
/// use jackdaw::prelude::*;
/// use std::time::Duration;
///
/// assert_eq!(500.millis(), Duration::from_millis(500));
/// assert_eq!(30.secs(), Duration::from_secs(30));
/// assert_eq!(5.minutes(), Duration::from_secs(300));
/// assert_eq!(24.hours(), Duration::from_secs(86_400));
/// assert_eq!(2.days(), Duration::from_secs(172_800));
/// ```
pub trait DurationExt {
    /// This many milliseconds.
    fn millis(self) -> Duration;
    /// This many seconds.
    fn secs(self) -> Duration;
    /// This many minutes.
    fn minutes(self) -> Duration;
    /// This many hours.
    fn hours(self) -> Duration;
    /// This many days of 24 hours.
    fn days(self) -> Duration;
}

macro_rules! impl_duration_ext {
    ($($ty:ty),*) => {$(
        impl DurationExt for $ty {
            fn millis(self) -> Duration { Duration::from_millis(u64::try_from(self).unwrap_or(0)) }
            fn secs(self) -> Duration { Duration::from_secs(u64::try_from(self).unwrap_or(0)) }
            fn minutes(self) -> Duration { Duration::from_secs(60 * u64::try_from(self).unwrap_or(0)) }
            fn hours(self) -> Duration { Duration::from_secs(3600 * u64::try_from(self).unwrap_or(0)) }
            fn days(self) -> Duration { Duration::from_secs(86_400 * u64::try_from(self).unwrap_or(0)) }
        }
    )*};
}

impl_duration_ext!(i32, i64, u32, u64, usize, u8, u16, i8, i16, isize);
