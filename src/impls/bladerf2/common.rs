//! Helpers shared by the synchronous and asynchronous bladeRF 2.0 backends.

use std::ops::RangeInclusive;
use std::time::Duration;

use crate::{Args, Direction, Error, Range, RangeItem};

pub(super) fn bladerf2_channel(
    direction: Direction,
    channel: usize,
) -> Result<bladerf2::Channel, Error> {
    match (direction, channel) {
        (Direction::Rx, 0) => Ok(bladerf2::Channel::Rx0),
        (Direction::Rx, 1) => Ok(bladerf2::Channel::Rx1),
        (Direction::Tx, 0) => Ok(bladerf2::Channel::Tx0),
        (Direction::Tx, 1) => Ok(bladerf2::Channel::Tx1),
        _ => Err(Error::invalid_channel(direction, channel, 2)),
    }
}

pub(super) fn range_u64(range: &RangeInclusive<u64>) -> Range {
    Range::new(vec![RangeItem::Interval(
        *range.start() as f64,
        *range.end() as f64,
    )])
    .expect("valid bladeRF range")
}

pub(super) fn range_f64(range: &RangeInclusive<f64>) -> Range {
    Range::new(vec![RangeItem::Interval(*range.start(), *range.end())])
        .expect("valid bladeRF range")
}

/// Map seify's per-call `timeout_us` (negative = infinite) to a duration.
pub(super) fn timeout(timeout_us: i64) -> Option<Duration> {
    (timeout_us >= 0).then(|| Duration::from_micros(timeout_us as u64))
}

/// Which device to open, from the `index` / `serial` arguments.
pub(super) enum DeviceSelector {
    /// The first device found.
    First,
    /// The device at this position in the enumeration order.
    Index(usize),
    /// The device whose serial number starts with this string.
    Serial(String),
}

impl DeviceSelector {
    fn matches(&self, index: usize, dev: &bladerf2::DeviceInfo) -> bool {
        match self {
            Self::First => true,
            Self::Index(i) => index == *i,
            Self::Serial(s) => dev.serial_number().is_some_and(|d| d.starts_with(s.as_str())),
        }
    }
}

pub(super) fn device_selector(args: &Args) -> Result<DeviceSelector, Error> {
    match args.get::<usize>("index") {
        Ok(index) => return Ok(DeviceSelector::Index(index)),
        Err(Error::MissingArgument { .. }) => {}
        Err(error) => return Err(error),
    }
    match args.get::<String>("serial") {
        Ok(serial) => Ok(DeviceSelector::Serial(serial)),
        Err(Error::MissingArgument { .. }) => Ok(DeviceSelector::First),
        Err(error) => Err(error),
    }
}

pub(super) fn boolean_arg(args: &Args, name: &str) -> Result<Option<bool>, Error> {
    let Ok(value) = args.get::<String>(name) else {
        return Ok(None);
    };
    if ["true", "on", "1"].contains(&value.as_str()) {
        Ok(Some(true))
    } else if ["false", "off", "0"].contains(&value.as_str()) {
        Ok(Some(false))
    } else {
        Err(Error::invalid_argument(name, "value must be 1 or 0"))
    }
}

/// Describe the devices matching `selector` as seify [`Args`].
pub(super) fn probe_args(
    selector: &DeviceSelector,
    devices: Vec<bladerf2::DeviceInfo>,
) -> Result<Vec<Args>, Error> {
    Ok(devices
        .into_iter()
        .enumerate()
        .filter(|(index, dev)| selector.matches(*index, dev))
        .map(|(_, dev)| {
            let mut args = Args::default();
            args.set("driver", "bladerf2");
            if let Some(serial) = dev.serial_number() {
                args.set("serial", serial);
            }
            args
        })
        .collect())
}

/// Pick the device matching `selector` out of an enumeration.
pub(super) fn select_device(
    selector: &DeviceSelector,
    devices: Vec<bladerf2::DeviceInfo>,
) -> Result<bladerf2::DeviceInfo, Error> {
    devices
        .into_iter()
        .enumerate()
        .find(|(index, dev)| selector.matches(*index, dev))
        .map(|(_, dev)| dev)
        .ok_or(Error::DeviceNotFound)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn webusb_filters(args: &Args) -> Result<Vec<crate::dev::WebUsbDeviceFilter>, Error> {
    let serial = match device_selector(args)? {
        DeviceSelector::Serial(serial) => Some(serial),
        _ => None,
    };
    let filter = crate::dev::WebUsbDeviceFilter::new().with_vendor_product(0x2CF0, 0x5250);
    Ok(vec![match serial {
        Some(serial) => filter.with_serial_number(serial),
        None => filter,
    }])
}

impl From<Direction> for bladerf2::Direction {
    fn from(value: Direction) -> Self {
        match value {
            Direction::Rx => bladerf2::Direction::Rx,
            Direction::Tx => bladerf2::Direction::Tx,
        }
    }
}
