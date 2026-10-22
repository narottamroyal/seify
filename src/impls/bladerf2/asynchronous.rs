//! bladeRF 2.0 micro backend (async: native via `smol`/`tokio`, wasm via WebUSB).

use futures::lock::Mutex;
use num_complex::Complex32;

use super::common::*;
use crate::async_compat::Shared;
use crate::dev::AsyncTypedDeviceBackend;
use crate::{
    Args, AsyncAgcControl, AsyncBandwidthControl, AsyncDeviceInfo, AsyncFrequencyControl,
    AsyncGainControl, AsyncRxDevice, AsyncSampleRateControl, AsyncTxDevice, Capability, Direction,
    Driver, Error, Range,
};

/// bladeRF 2.0 micro device (async).
///
/// Cloning is cheap: clones share one device. A [`bladerf2::RfLinkSession`]
/// needs exclusive access, so the device sits behind an async mutex and
/// control calls from different tasks queue up. Streamers own their USB
/// endpoints and never take the lock, so receiving samples is never blocked
/// by a control call.
#[derive(Clone)]
pub struct AsyncBladeRf2 {
    device: Shared<Mutex<bladerf2::BladeRf2>>,
    serial: String,
}

impl AsyncBladeRf2 {
    /// Return descriptors for the devices this process can open.
    ///
    /// On wasm only devices the browser has granted are listed.
    pub async fn probe(args: &Args) -> Result<Vec<Args>, Error> {
        let devices = bladerf2::BladeRf2::list_devices().await?;
        probe_args(&device_selector(args)?, devices)
    }

    /// Open a device (`driver=bladerf2`, optional `serial`, `index`,
    /// `biastee_rx`, `biastee_tx`).
    ///
    /// On wasm the registry has already asked the browser for permission, so
    /// only granted devices are visible here.
    pub async fn open<A: TryInto<Args>>(args: A) -> Result<Self, Error> {
        let args = args
            .try_into()
            .map_err(|_| Error::invalid_argument("args", "failed to convert args"))?;

        let devices = bladerf2::BladeRf2::list_devices().await?;
        let info = select_device(&device_selector(&args)?, devices)?;
        let mut device = bladerf2::BladeRf2::open_device(info).await?;
        let serial = device.serial()?.to_string();

        for (name, direction) in [
            ("biastee_rx", bladerf2::Direction::Rx),
            ("biastee_tx", bladerf2::Direction::Tx),
        ] {
            if let Some(value) = boolean_arg(&args, name)? {
                device
                    .rf_link_session()
                    .set_bias_tee(direction, value)
                    .await?;
            }
        }

        Ok(Self {
            device: Shared::new(Mutex::new(device)),
            serial,
        })
    }

    /// Run `operation` with exclusive access to a fresh RF-link session.
    async fn with_session<T>(
        &self,
        operation: impl AsyncFnOnce(&bladerf2::RfLinkSession<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut device = self.device.lock().await;
        let rf = device.rf_link_session();
        operation(&rf).await
    }
}

impl AsyncDeviceInfo for AsyncBladeRf2 {
    fn driver(&self) -> Driver {
        Driver::BladeRf2
    }

    async fn async_id(&self) -> Result<String, Error> {
        Ok(self.serial.clone())
    }

    async fn async_info(&self) -> Result<Args, Error> {
        format!("driver=bladerf2, serial={}", self.serial).try_into()
    }

    async fn async_num_channels(&self, _direction: Direction) -> Result<usize, Error> {
        Ok(2)
    }

    async fn async_full_duplex(&self) -> Result<bool, Error> {
        Ok(true)
    }
}

crate::impl_dyn_async_device_backend!(
    AsyncBladeRf2 => [rx, tx, agc, gain, frequency, sample_rate, bandwidth]
);

impl AsyncRxDevice for AsyncBladeRf2 {
    type RxStreamer = AsyncBladeRf2RxStreamer;

    async fn async_rx_streamer(
        &self,
        channels: &[usize],
        _args: Args,
    ) -> Result<Self::RxStreamer, Error> {
        let stream = self
            .with_session(async |rf| Ok(rf.rx_stream(channels)?))
            .await?;
        Ok(AsyncBladeRf2RxStreamer {
            stream,
            active: false,
        })
    }
}

impl AsyncTxDevice for AsyncBladeRf2 {
    type TxStreamer = AsyncBladeRf2TxStreamer;

    async fn async_tx_streamer(
        &self,
        channels: &[usize],
        _args: Args,
    ) -> Result<Self::TxStreamer, Error> {
        let stream = self
            .with_session(async |rf| Ok(rf.tx_stream(channels)?))
            .await?;
        Ok(AsyncBladeRf2TxStreamer {
            stream,
            active: false,
        })
    }
}

impl AsyncFrequencyControl for AsyncBladeRf2 {
    async fn async_frequency_range(
        &self,
        direction: Direction,
        channel: usize,
    ) -> Result<Range, Error> {
        bladerf2_channel(direction, channel)?;
        let range = self
            .with_session(async |rf| Ok(rf.frequency_range(direction.into())))
            .await?;
        Ok(range_u64(&range))
    }

    async fn async_frequency(&self, direction: Direction, channel: usize) -> Result<f64, Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| Ok(rf.frequency(direction.into()).await? as f64))
            .await
    }

    async fn async_set_frequency(
        &self,
        direction: Direction,
        channel: usize,
        frequency: f64,
        _args: Args,
    ) -> Result<(), Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| {
            Ok(rf.set_frequency(direction.into(), frequency as u64).await?)
        })
        .await
    }

    async fn async_frequency_components(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Vec<String>, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    async fn async_component_frequency_range(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Range, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    async fn async_component_frequency(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<f64, Error> {
        Err(Error::unsupported(Capability::Frequency))
    }

    async fn async_set_component_frequency(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
        _frequency: f64,
    ) -> Result<(), Error> {
        Err(Error::unsupported(Capability::Frequency))
    }
}

impl AsyncSampleRateControl for AsyncBladeRf2 {
    async fn async_sample_rate(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<f64, Error> {
        self.with_session(async |rf| Ok(rf.sample_rate().await? as f64))
            .await
    }

    async fn async_set_sample_rate(
        &self,
        _direction: Direction,
        _channel: usize,
        rate: f64,
    ) -> Result<(), Error> {
        self.with_session(async |rf| Ok(rf.set_sample_rate(rate as u64).await?))
            .await
    }

    async fn async_get_sample_rate_range(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Range, Error> {
        let range = self
            .with_session(async |rf| Ok(rf.sample_rate_range()))
            .await?;
        Ok(range_u64(&range))
    }
}

impl AsyncBandwidthControl for AsyncBladeRf2 {
    async fn async_bandwidth(&self, direction: Direction, channel: usize) -> Result<f64, Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| Ok(rf.bandwidth(direction.into()).await? as f64))
            .await
    }

    async fn async_set_bandwidth(
        &self,
        direction: Direction,
        channel: usize,
        bw: f64,
    ) -> Result<(), Error> {
        bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| Ok(rf.set_bandwidth(direction.into(), bw as u64).await?))
            .await
    }

    async fn async_get_bandwidth_range(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Range, Error> {
        let range = self
            .with_session(async |rf| Ok(rf.bandwidth_range()))
            .await?;
        Ok(range_u64(&range))
    }
}

impl AsyncGainControl for AsyncBladeRf2 {
    async fn async_gain_elements(
        &self,
        _direction: Direction,
        _channel: usize,
    ) -> Result<Vec<String>, Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    async fn async_set_gain(
        &self,
        direction: Direction,
        channel: usize,
        gain: f64,
    ) -> Result<(), Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| Ok(rf.set_gain(channel, gain).await?))
            .await
    }

    async fn async_gain(&self, direction: Direction, channel: usize) -> Result<Option<f64>, Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| Ok(Some(rf.gain(channel).await?)))
            .await
    }

    async fn async_gain_range(&self, direction: Direction, channel: usize) -> Result<Range, Error> {
        bladerf2_channel(direction, channel)?;
        let range = self
            .with_session(async |rf| Ok(rf.gain_range(direction.into()).await?))
            .await?;
        Ok(range_f64(&range.gain_range))
    }

    async fn async_set_gain_element(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
        _gain: f64,
    ) -> Result<(), Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    async fn async_gain_element(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Option<f64>, Error> {
        Err(Error::unsupported(Capability::Gain))
    }

    async fn async_gain_element_range(
        &self,
        _direction: Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Range, Error> {
        Err(Error::unsupported(Capability::Gain))
    }
}

impl AsyncAgcControl for AsyncBladeRf2 {
    async fn async_agc_available(
        &self,
        direction: Direction,
        channel: usize,
    ) -> Result<bool, Error> {
        Ok(direction == Direction::Rx && channel < 2)
    }

    async fn async_set_agc_enabled(
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
        match direction {
            Direction::Rx => {
                self.with_session(async |rf| Ok(rf.set_gain_mode(channel, mode).await?))
                    .await
            }
            Direction::Tx => Err(Error::unsupported_reason(Capability::Agc, "TX has no AGC")),
        }
    }

    async fn async_agc_enabled(&self, direction: Direction, channel: usize) -> Result<bool, Error> {
        let channel = bladerf2_channel(direction, channel)?;
        self.with_session(async |rf| match direction {
            Direction::Rx => {
                let mode = rf.gain_mode(channel).await?;
                Ok(mode != bladerf2::GainMode::Manual)
            }
            Direction::Tx => Err(Error::unsupported_reason(Capability::Agc, "TX has no AGC")),
        })
        .await
    }
}

/// bladeRF 2.0 async receive streamer.
///
/// Call `deactivate` before dropping it; dropping an active streamer only
/// deactivates the radio on a best-effort basis (see [`bladerf2::RxStream`]).
#[must_use = "deactivate the streamer before dropping it"]
pub struct AsyncBladeRf2RxStreamer {
    stream: bladerf2::RxStream,
    active: bool,
}

impl crate::AsyncRxStreamer for AsyncBladeRf2RxStreamer {
    fn mtu(
        &self,
    ) -> impl std::future::Future<Output = Result<usize, Error>> + crate::MaybeSend + '_ {
        std::future::ready(Ok(self.stream.mtu()))
    }

    async fn activate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedActivation));
        }
        if !self.active {
            self.stream.activate().await?;
            self.active = true;
        }
        Ok(())
    }

    async fn deactivate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedDeactivation));
        }
        if self.active {
            self.stream.deactivate().await?;
            self.active = false;
        }
        Ok(())
    }

    async fn read<'a>(
        &'a mut self,
        buffers: &'a mut [&'a mut [Complex32]],
        timeout_us: i64,
    ) -> Result<usize, Error> {
        if !self.active {
            return Err(Error::StreamInactive);
        }
        let n = buffers.iter().map(|b| b.len()).min().unwrap_or(0);
        if n == 0 {
            return Ok(0);
        }
        let mut sub: Vec<&mut [Complex32]> = buffers.iter_mut().map(|b| &mut b[..n]).collect();
        self.stream
            .read_async(&mut sub, timeout(timeout_us))
            .await?;
        Ok(n)
    }
}

/// bladeRF 2.0 async transmit streamer.
///
/// Call `deactivate` before dropping it; dropping an active streamer only
/// deactivates the radio on a best-effort basis (see [`bladerf2::TxStream`]).
#[must_use = "deactivate the streamer before dropping it"]
pub struct AsyncBladeRf2TxStreamer {
    stream: bladerf2::TxStream,
    active: bool,
}

impl crate::AsyncTxStreamer for AsyncBladeRf2TxStreamer {
    fn mtu(
        &self,
    ) -> impl std::future::Future<Output = Result<usize, Error>> + crate::MaybeSend + '_ {
        std::future::ready(Ok(self.stream.mtu()))
    }

    async fn activate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedActivation));
        }
        if !self.active {
            self.stream.activate().await?;
            self.active = true;
        }
        Ok(())
    }

    async fn deactivate_at(&mut self, time_ns: Option<i64>) -> Result<(), Error> {
        if time_ns.is_some() {
            return Err(Error::unsupported(Capability::TimedDeactivation));
        }
        if self.active {
            self.stream.deactivate().await?;
            self.active = false;
        }
        Ok(())
    }

    async fn write<'a>(
        &'a mut self,
        buffers: &'a [&'a [Complex32]],
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
        self.stream.write_async(&sub, timeout(timeout_us)).await?;
        Ok(n)
    }
}

impl AsyncTypedDeviceBackend for AsyncBladeRf2 {
    fn driver() -> Driver {
        Driver::BladeRf2
    }

    #[cfg(target_arch = "wasm32")]
    fn webusb_filters(args: &Args) -> Result<Vec<crate::dev::WebUsbDeviceFilter>, Error> {
        webusb_filters(args)
    }

    async fn async_probe(args: &Args) -> Result<Vec<Args>, Error> {
        Self::probe(args).await
    }

    async fn async_open(args: &Args) -> Result<Self, Error> {
        Self::open(args.clone()).await
    }
}
