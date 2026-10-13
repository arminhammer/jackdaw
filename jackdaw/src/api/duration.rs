//! `10.millis()`, `30.secs()`, `24.hours()`: readable durations on integer literals.
use std::time::Duration;

pub trait DurationExt {
    fn millis(self) -> Duration;
    fn secs(self) -> Duration;
    fn minutes(self) -> Duration;
    fn hours(self) -> Duration;
}

macro_rules! impl_duration_ext {
    ($($ty:ty),*) => {$(
        impl DurationExt for $ty {
            fn millis(self) -> Duration { Duration::from_millis(u64::try_from(self).unwrap_or(0)) }
            fn secs(self) -> Duration { Duration::from_secs(u64::try_from(self).unwrap_or(0)) }
            fn minutes(self) -> Duration { Duration::from_secs(60 * u64::try_from(self).unwrap_or(0)) }
            fn hours(self) -> Duration { Duration::from_secs(3600 * u64::try_from(self).unwrap_or(0)) }
        }
    )*};
}

impl_duration_ext!(i32, i64, u32, u64, usize, u8, u16, i8, i16, isize);
