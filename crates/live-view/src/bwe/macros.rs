// str0m writes these to stdout for its own analysis under a feature of its
// test builds; here they compile to nothing, and the estimate is the Host
// log's when it changes the encoding.
macro_rules! log_stat {
    ($name:expr, $($arg:expr),+) => {};
}
pub(crate) use log_stat;

macro_rules! log_delay_variation {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("DELAY_VARIATION", $($arg),+);
    }
}

macro_rules! log_trendline_estimate {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("TRENDLINE_ESTIMATE", $($arg),+);
    }
}

macro_rules! log_trendline_modified_trend {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("TRENDLINE_MODIFIED_TREND", $($arg),+);
    }
}

macro_rules! log_bitrate_estimate {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("BITRATE_ESTIMATE", $($arg),+);
    }
}

macro_rules! log_rate_control_state {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("RATE_CONTROL_STATE", $($arg),+);
    }
}

macro_rules! log_rate_control_observed_bitrate {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("RATE_CONTROL_OBSERVED_BITRATE", $($arg),+);
    }
}

macro_rules! log_rate_control_applied_change {
    ($($arg:expr),+) => {
        $crate::bwe::macros::log_stat!("RATE_CONTROL_APPLIED_CHANGE", $($arg),+);
    }
}

pub(crate) use log_bitrate_estimate;

pub(crate) use log_delay_variation;

pub(crate) use log_rate_control_applied_change;

pub(crate) use log_rate_control_observed_bitrate;

pub(crate) use log_rate_control_state;

pub(crate) use log_trendline_estimate;

pub(crate) use log_trendline_modified_trend;
