//! bladeRF 2.0 micro backend (synchronous, native only).

use std::sync::{Arc, Mutex};

use bladerf2::MaybeFuture;
use num_complex::Complex32;

use super::common::*;
use crate::{
    AgcControl, Args, BandwidthControl, Capability, DeviceInfo, Direction, Driver, Error,
    FrequencyControl, GainControl, Range, RxDevice, SampleRateControl, TxDevice,
};

/// bladeRF 2.0 micro device.
#[derive(Clone)]
pub struct BladeRf2 {
    inner: Arc<Mutex<bladerf2::BladeRf2>>,
    serial: String,
}

impl BladeRf2 {
    /// Return descriptors for detected devices.
    pub fn probe(args: &Args) -> Result<Vec<Args>, Error> {
        let devices = bladerf2::BladeRf2::enumerate()?;
        probe_args(&device_selector(args)?, devices)
    }

    /// Open a device (`driver=bladerf2`, optional `serial`, `index`,
    /// `biastee_rx`, `biastee_tx`).
    pub fn open<A: TryInto<Args>>(args: A) -> Result<Self, Error> {
        let args = args
            .try_into()
            .map_err(|_| Error::invalid_argument("args", "failed to convert args"))?;
        let devices = bladerf2::BladeRf2::enumerate()?;
        let info = select_device(&device_selector(&args)?, devices)?;
        let mut device = bladerf2::BladeRf2::open(info)?;
        let serial = device.serial()?.to_string();

        for (name, direction) in [
            ("biastee_rx", bladerf2::Direction::Rx),
            ("biastee_tx", bladerf2::Direction::Tx),
        ] {
            if let Some(value) = boolean_arg(&args, name)? {
                device
                    .rf_link_session()
                    .set_bias_tee(direction, value)
                    .wait()?;
            }
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(device)),
            serial,
        })
    }

    /// Run `operation` with exclusive access to a fresh RF-link session.
    ///
    /// The mutex serializes control calls from different threads; streamers
    /// own their USB endpoints and never take it.
    fn with_session<T>(
        &self,
        operation: impl FnOnce(&bladerf2::RfLinkSession<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut dev = self.inner.lock().map_err(|_| Error::Busy)?;
        let rf = dev.rf_link_session();
        operation(&rf)
    }
}

impl DeviceInfo for BladeRf2 {
    fn driver(&self) -> Driver {
        Driver::BladeRf2
    }

    fn id(&self) -> Result<String, Error> {
        Ok(self.serial.clone())
    }

    fn info(&self) -> Result<Args, Error> {
        format!("driver=bladerf2, serial={}", self.serial).try_into()
    }

    fn num_channels(&self, _direction: Direction) -> Result<usize, Error> {
        Ok(2)
    }

    fn full_duplex(&self) -> Result<bool, Error> {
        Ok(true)
    }
}

crate::impl_dyn_device_backend!(BladeRf2 => [rx, tx, agc, gain, frequency, sample_rate, bandwidth]);
crate::registry::impl_typed_device_backend!(BladeRf2, Driver::BladeRf2);

impl RxDevice for BladeRf2 {
    type RxStreamer = RxStreamer;

    fn rx_streamer(&self, channels: &[usize], _args: Args) -> Result<Self::RxStreamer, Error> {
        let stream = self.with_session(|s| Ok(s.rx_stream(channels)?))?;
        Ok(RxStreamer {
            stream,
            active: false,
        })
    }
}

impl TxDevice for BladeRf2 {
    type TxStreamer = TxStreamer;

    fn tx_streamer(&self, channels: &[usize], _args: Args) -> Result<Self::TxStreamer, Error> {
        let stream = self.with_session(|s| Ok(s.tx_stream(channels)?))?;
        Ok(TxStreamer {
            stream,
            active: false,
        })
    }
}

impl AgcControl for BladeRf2 {
    fn agc_available(&self, direction: Direction, channel: usize) -> Result<bool, Error> {
        Ok(direction == Direction::Rx && channel < 2)
    }

    fn set_agc_enabled(
        &self,
        direction: Direction,
        channel: usize,
        enabled: bool,
    ) -> Result<(), Error> {
        let channel = bladerf2_channel(direction, channel)?;
        let mode = if enabled {
            bladerf2::GainMode::default()
        } else {
            bladerf2::GainMode::Manual
        };
        self.with_session(|s| match direction {
            Direction::Rx => Ok(s.set_gain_mode(channel, mode).wait()?),
            Direction::Tx => Err(Error::unsupported_reason(Capability::Agc, "TX has no AGC")),
        })
    }

    fn agc_enabled(&self, direction: Direction, channel: usize) -> Result<bool, Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(|s| match direction {
            Direction::Rx => {
                let mode = s.gain_mode(channel).wait()?;
                Ok(mode != bladerf2::GainMode::Manual)
            }
            Direction::Tx => Err(Error::unsupported_reason(Capability::Agc, "TX has no AGC")),
        })
    }
}

impl GainControl for BladeRf2 {
    fn gain_elements(&self, _direction: Direction, _channel: usize) -> Result<Vec<String>, Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    fn set_gain(&self, direction: Direction, channel: usize, gain: f64) -> Result<(), Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(s.set_gain(channel, gain).wait()?))
    }

    fn gain(&self, direction: Direction, channel: usize) -> Result<Option<f64>, Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(Some(s.gain(channel).wait()?)))
    }

    fn gain_range(&self, direction: Direction, channel: usize) -> Result<Range, Error> {
        bladerf2_channel(direction, channel)?;
        let range = self.with_session(|s| Ok(s.gain_range(direction.into()).wait()?))?;
        Ok(range_f64(&range.gain_range))
    }

    fn set_gain_element(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
        _gain: f64,
    ) -> Result<(), Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    fn gain_element(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Option<f64>, Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    fn gain_element_range(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Range, Error> {
        Err(Error::unsupported(Capability::Gain))
    }
}

impl FrequencyControl for BladeRf2 {
    fn frequency_range(&self, direction: Direction, channel: usize) -> Result<Range, Error> {
        bladerf2_channel(direction, channel)?;
        let range = self.with_session(|s| Ok(s.frequency_range(direction.into())))?;
        Ok(range_u64(&range))
    }

    fn frequency(&self, direction: Direction, channel: usize) -> Result<f64, Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(s.frequency(direction.into()).wait()? as f64))
    }

    fn set_frequency(
        &self,
        direction: Direction,
        channel: usize,
        frequency: f64,
        _args: Args,
    ) -> Result<(), Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(s.set_frequency(direction.into(), frequency as u64).wait()?))
    }

    fn frequency_components(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Vec<String>, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    fn component_frequency_range(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Range, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    fn component_frequency(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<f64, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    fn set_component_frequency(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
        _frequency: f64,
    ) -> Result<(), Error> {
        Err(Error::unsupported(Capability::Frequency))
    }
}

impl SampleRateControl for BladeRf2 {
    fn sample_rate(&self, _direction: Direction, _channel: usize) -> Result<f64, Error> {
        self.with_session(|s| Ok(s.sample_rate().wait()? as f64))
    }

    fn set_sample_rate(
        &self,
        _direction: Direction,
        _channel: usize,
        rate: f64,
    ) -> Result<(), Error> {
        self.with_session(|s| Ok(s.set_sample_rate(rate as u64).wait()?))
    }

    fn get_sample_rate_range(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Range, Error> {
        let range = self.with_session(|s| Ok(s.sample_rate_range()))?;
        Ok(range_u64(&range))
    }
}

impl BandwidthControl for BladeRf2 {
    fn bandwidth(&self, direction: Direction, channel: usize) -> Result<f64, Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(s.bandwidth(direction.into()).wait()? as f64))
    }

    fn set_bandwidth(&self, direction: Direction, channel: usize, bw: f64) -> Result<(), Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(|s| Ok(s.set_bandwidth(direction.into(), bw as u64).wait()?))
    }

    fn get_bandwidth_range(&self, _direction: Direction, _channel: usize) -> Result<Range, Error> {
        let range = self.with_session(|s| Ok(s.bandwidth_range()))?;
        Ok(range_u64(&range))
    }
}

/// bladeRF 2.0 receive streamer.
pub struct RxStreamer {
    stream: bladerf2::RxStream,
    active: bool,
}

impl RxStreamer {
    fn stop(&mut self) -> Result<(), Error> {
        if self.active {
            self.stream.deactivate().wait()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for RxStreamer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl crate::RxStreamer for RxStreamer {
    fn mtu(&self) -> Result<usize, Error> {
        Ok(self.stream.mtu())
    }

    fn activate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedActivation));
        }
        if !self.active {
            self.stream.activate().wait()?;
            self.active = true;
        }
        Ok(())
    }

    fn deactivate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedDeactivation));
        }
        self.stop()
    }

    fn read(&mut self, buffers: &mut [&mut [Complex32]], timeout_us: i64) -> Result<usize, Error> {
        if !self.active {
            return Err(Error::StreamInactive);
        }
        let n = buffers.iter().map(|b| b.len()).min().unwrap_or(0);
        if n == 0 {
            return Ok(0);
        }
        let mut sub: Vec<&mut [Complex32]> = buffers.iter_mut().map(|b| &mut b[..n]).collect();
        self.stream.read(&mut sub, timeout(timeout_us))?;
        Ok(n)
    }
}

/// bladeRF 2.0 transmit streamer.
pub struct TxStreamer {
    stream: bladerf2::TxStream,
    active: bool,
}

impl TxStreamer {
    fn stop(&mut self) -> Result<(), Error> {
        if self.active {
            self.stream.deactivate().wait()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for TxStreamer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl crate::TxStreamer for TxStreamer {
    fn mtu(&self) -> Result<usize, Error> {
        Ok(self.stream.mtu())
    }

    fn activate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedActivation));
        }
        if !self.active {
            self.stream.activate().wait()?;
            self.active = true;
        }
        Ok(())
    }

    fn deactivate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedDeactivation));
        }
        self.stop()
    }

    fn write(
        &mut self,
        buffers: &[&[Complex32]],
        _at_ns: Option<i64>,
        _end_burst: bool,
        timeout_us: i64,
    ) -> Result<usize, Error> {
        if !self.active {
            return Err(Error::StreamInactive);
        }
        let n = buffers.iter().map(|b| b.len()).min().unwrap_or(0);
        if n == 0 {
            return Ok(0);
        }
        let sub: Vec<&[Complex32]> = buffers.iter().map(|b| &b[..n]).collect();
        self.stream.write(&sub, timeout(timeout_us))?;
        Ok(n)
    }

    fn write_all(
        &mut self,
        buffers: &[&[Complex32]],
        at_ns: Option<i64>,
        end_burst: bool,
        timeout_us: i64,
    ) -> Result<(), Error> {
        self.write(buffers, at_ns, end_burst, timeout_us)
            .map(|_| ())
    }
}
