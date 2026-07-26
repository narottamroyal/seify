use crate::{
    dev::DynDeviceBackend, AgcControl, AntennaControl, Args, BandwidthControl, Capability,
    ChannelInfo, DeviceInfo, Direction, Error, FrequencyControl, GainControl, RxDevice, RxStreamer,
    SampleRateControl, TxDevice, TxStreamer,
};

impl From<crate::Direction> for bladerf2::Direction {
    fn from(direction: crate::Direction) -> Self {
        match direction {
            crate::Direction::Rx => bladerf2::Direction::Rx,
            crate::Direction::Tx => bladerf2::Direction::Tx,
        }
    }
}

/// bladeRF 2 device backend.
#[derive(Clone)]
pub struct BladeRf2 {
    inner: bladerf2::BladeRf2,
}

impl BladeRf2 {
    /// Return list of detected bladeRF 2 devices.
    pub fn probe(_args: &Args) -> Result<Vec<Args>, Error> {
        let mut devices = vec![];
        for dev in bladerf2::BladeRf2::enumerate()? {
            let serial = dev.serial_number().unwrap_or_default();
            devices.push(format!("driver=bladerf2, serial={serial}").try_into()?);
        }
        Ok(devices)
    }

    /// Open a bladeRF 2 device with arguments.
    pub fn open<A: TryInto<Args>>(args: A) -> Result<Self, Error> {
        let args = args
            .try_into()
            .map_err(|_| Error::invalid_argument("args", "failed to convert args"))?;
        let serial = args.get::<String>("serial").unwrap_or_default();
        let bladerf2 = bladerf2::BladeRf2::open_serial(&serial)?;
        let biastee = |name: &str, value: &str| {
            if ["true", "on", "1"].contains(&value) {
                Ok(true)
            } else if ["false", "off", "0"].contains(&value) {
                Ok(false)
            } else {
                Err(crate::Error::invalid_argument(name, "value must be 1 or 0"))
            }
        };
        if let Ok(value) = args.get::<String>("biastee_rx") {
            bladerf2.set_bias_tee(bladerf2::Direction::Rx, biastee("biastee_rx", &value)?)?;
        }
        if let Ok(value) = args.get::<String>("biastee_tx") {
            bladerf2.set_bias_tee(bladerf2::Direction::Tx, biastee("biastee_tx", &value)?)?;
        }
        Ok(Self { inner: bladerf2 })
    }
}

impl DeviceInfo for BladeRf2 {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn driver(&self) -> crate::Driver {
        crate::Driver::BladeRf2
    }

    fn id(&self) -> Result<String, Error> {
        Ok(self.inner.serial()?.to_string())
    }

    fn info(&self) -> Result<Args, Error> {
        let serial = self.inner.serial()?;
        format!("driver=bladerf2, serial={serial}").try_into()
    }
}

impl DynDeviceBackend for BladeRf2 {
    fn channel_info(&self) -> Option<&dyn ChannelInfo> {
        Some(self)
    }

    fn rx_device(&self) -> Option<&dyn crate::dev::ErasedRxDevice> {
        Some(self)
    }

    fn tx_device(&self) -> Option<&dyn crate::dev::ErasedTxDevice> {
        Some(self)
    }

    fn antenna_control(&self) -> Option<&dyn AntennaControl> {
        None
    }

    fn agc_control(&self) -> Option<&dyn AgcControl> {
        Some(self)
    }

    fn gain_control(&self) -> Option<&dyn GainControl> {
        Some(self)
    }

    fn frequency_control(&self) -> Option<&dyn FrequencyControl> {
        Some(self)
    }

    fn sample_rate_control(&self) -> Option<&dyn SampleRateControl> {
        Some(self)
    }

    fn bandwidth_control(&self) -> Option<&dyn BandwidthControl> {
        Some(self)
    }
}

impl ChannelInfo for BladeRf2 {
    fn num_channels(&self, _direction: crate::Direction) -> Result<usize, Error> {
        Ok(2)
    }

    fn full_duplex(&self, _direction: crate::Direction, _channel: usize) -> Result<bool, Error> {
        Ok(true)
    }
}

impl RxDevice for BladeRf2 {
    type RxStreamer = bladerf2::RxStream;

    fn rx_streamer(&self, channels: &[usize], _args: Args) -> Result<Self::RxStreamer, Error> {
        Ok(self.inner.rx_stream(channels)?)
    }
}

impl TxDevice for BladeRf2 {
    type TxStreamer = bladerf2::TxStream;

    fn tx_streamer(&self, channels: &[usize], _args: Args) -> Result<Self::TxStreamer, Error> {
        Ok(self.inner.tx_stream(channels)?)
    }
}

impl AgcControl for BladeRf2 {
    fn agc_available(&self, direction: Direction, channel: usize) -> Result<bool, Error> {
        Ok(direction == crate::Direction::Rx && channel < 2)
    }

    fn agc_enabled(&self, direction: Direction, channel: usize) -> Result<bool, Error> {
        let channel = match channel {
            0 => bladerf2::Channel::Rx0,
            1 => bladerf2::Channel::Rx1,
            _ => return Err(crate::Error::invalid_channel(direction, channel, 2)),
        };

        match direction {
            crate::Direction::Rx => {
                let mode = self.inner.gain_mode(channel)?;
                Ok(mode == bladerf2::GainMode::Manual)
            }
            crate::Direction::Tx => Err(crate::Error::unsupported_reason(
                Capability::Agc,
                "TX does ",
            )),
        }
    }

    fn set_agc_enabled(
        &self,
        direction: Direction,
        channel: usize,
        enabled: bool,
    ) -> Result<(), Error> {
        let channel = match channel {
            0 => bladerf2::Channel::Rx0,
            1 => bladerf2::Channel::Rx1,
            _ => return Err(crate::Error::invalid_channel(direction, channel, 2)),
        };
        let mode = if enabled {
            bladerf2::GainMode::default()
        } else {
            bladerf2::GainMode::Manual
        };

        match direction {
            crate::Direction::Rx => Ok(self.inner.set_gain_mode(channel.into(), mode)?),
            crate::Direction::Tx => Err(crate::Error::unsupported_reason(
                Capability::Agc,
                "TX does not have AGC",
            )),
        }
    }
}

impl GainControl for BladeRf2 {
    fn gain_elements(
        &self,
        _direction: crate::Direction,
        _channel: usize,
    ) -> Result<Vec<String>, Error> {
        Err(crate::Error::unsupported(Capability::Gain))
    }

    fn set_gain(
        &self,
        direction: crate::Direction,
        channel: usize,
        gain: f64,
    ) -> Result<(), Error> {
        let channel = match (direction, channel) {
            (crate::Direction::Rx, 0) => bladerf2::Channel::Rx0,
            (crate::Direction::Rx, 1) => bladerf2::Channel::Rx1,
            (crate::Direction::Tx, 0) => bladerf2::Channel::Tx0,
            (crate::Direction::Tx, 1) => bladerf2::Channel::Tx1,
            _ => return Err(crate::Error::invalid_channel(direction, channel, 2)),
        };
        Ok(self.inner.set_gain(channel, gain)?)
    }

    fn gain(&self, direction: crate::Direction, channel: usize) -> Result<Option<f64>, Error> {
        let channel = match (direction, channel) {
            (crate::Direction::Rx, 0) => bladerf2::Channel::Rx0,
            (crate::Direction::Rx, 1) => bladerf2::Channel::Rx1,
            (crate::Direction::Tx, 0) => bladerf2::Channel::Tx0,
            (crate::Direction::Tx, 1) => bladerf2::Channel::Tx1,
            _ => return Err(crate::Error::invalid_channel(direction, channel, 2)),
        };
        Ok(Some(self.inner.gain(channel)?))
    }

    fn gain_range(
        &self,
        direction: crate::Direction,
        _channel: usize,
    ) -> Result<crate::Range, Error> {
        let range = self.inner.gain_range(direction.into())?;
        Ok(crate::Range::new(vec![crate::RangeItem::Interval(
            *range.start(),
            *range.end(),
        )]))
    }

    fn set_gain_element(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
        _gain: f64,
    ) -> Result<(), Error> {
        Err(crate::Error::unsupported(Capability::Gain))
    }

    fn gain_element(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<Option<f64>, Error> {
        Err(crate::Error::unsupported(Capability::Gain))
    }

    fn gain_element_range(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<crate::Range, Error> {
        Err(crate::Error::unsupported(Capability::Gain))
    }
}

impl FrequencyControl for BladeRf2 {
    fn frequency_range(
        &self,
        direction: crate::Direction,
        _channel: usize,
    ) -> Result<crate::Range, Error> {
        let range = self.inner.frequency_range(direction.into());
        Ok(crate::Range::new(vec![crate::RangeItem::Interval(
            *range.start() as _,
            *range.end() as _,
        )]))
    }

    fn frequency(&self, direction: crate::Direction, _channel: usize) -> Result<f64, Error> {
        Ok(self.inner.frequency(direction.into())? as _)
    }

    fn set_frequency(
        &self,
        direction: crate::Direction,
        _channel: usize,
        frequency: f64,
        _args: Args,
    ) -> Result<(), Error> {
        Ok(self.inner.set_frequency(direction.into(), frequency as _)?)
    }

    fn frequency_components(
        &self,
        _direction: crate::Direction,
        _channel: usize,
    ) -> Result<Vec<String>, Error> {
        Err(crate::Error::unsupported(Capability::Frequency))
    }

    fn component_frequency_range(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<crate::Range, Error> {
        Err(crate::Error::unsupported(Capability::Frequency))
    }

    fn component_frequency(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
    ) -> Result<f64, Error> {
        Err(crate::Error::unsupported(Capability::Frequency))
    }

    fn set_component_frequency(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        _name: &str,
        _frequency: f64,
    ) -> Result<(), Error> {
        Err(crate::Error::unsupported(Capability::Frequency))
    }
}

impl SampleRateControl for BladeRf2 {
    fn sample_rate(&self, _direction: crate::Direction, _channel: usize) -> Result<f64, Error> {
        Ok(self.inner.sample_rate()? as _)
    }

    fn set_sample_rate(
        &self,
        _direction: crate::Direction,
        _channel: usize,
        rate: f64,
    ) -> Result<(), Error> {
        Ok(self.inner.set_sample_rate(rate as _)?)
    }

    fn get_sample_rate_range(
        &self,
        _direction: crate::Direction,
        _channel: usize,
    ) -> Result<crate::Range, Error> {
        let range = self.inner.sample_rate_range();
        Ok(crate::Range::new(vec![crate::RangeItem::Interval(
            *range.start() as _,
            *range.end() as _,
        )]))
    }
}

impl BandwidthControl for BladeRf2 {
    fn bandwidth(&self, direction: crate::Direction, _channel: usize) -> Result<f64, Error> {
        Ok(self.inner.bandwidth(direction.into())? as _)
    }

    fn set_bandwidth(
        &self,
        direction: crate::Direction,
        _channel: usize,
        bw: f64,
    ) -> Result<(), Error> {
        Ok(self.inner.set_bandwidth(direction.into(), bw as _)?)
    }

    fn get_bandwidth_range(
        &self,
        _direction: crate::Direction,
        _channel: usize,
    ) -> Result<crate::Range, Error> {
        let range = self.inner.bandwidth_range();
        Ok(crate::Range::new(vec![crate::RangeItem::Interval(
            *range.start() as _,
            *range.end() as _,
        )]))
    }
}

impl RxStreamer for bladerf2::RxStream {
    fn mtu(&self) -> Result<usize, Error> {
        Ok(2048)
    }

    fn activate_at(&mut self, _time_ns: Option<i64>) -> Result<(), Error> {
        Ok(self.activate()?)
    }

    fn deactivate_at(&mut self, _time_ns: Option<i64>) -> Result<(), Error> {
        Ok(self.deactivate()?)
    }

    fn read(
        &mut self,
        buffers: &mut [&mut [num_complex::Complex32]],
        _timeout_us: i64,
    ) -> Result<usize, Error> {
        self.read(buffers)?;
        Ok(buffers[0].len())
    }
}

impl TxStreamer for bladerf2::TxStream {
    fn mtu(&self) -> Result<usize, Error> {
        Ok(2048)
    }

    fn activate_at(&mut self, _time_ns: Option<i64>) -> Result<(), Error> {
        Ok(self.activate()?)
    }

    fn deactivate_at(&mut self, _time_ns: Option<i64>) -> Result<(), Error> {
        Ok(self.deactivate()?)
    }

    fn write(
        &mut self,
        buffers: &[&[num_complex::Complex32]],
        _at_ns: Option<i64>,
        _end_burst: bool,
        _timeout_us: i64,
    ) -> Result<usize, Error> {
        self.write(buffers)?;
        Ok(buffers[0].len())
    }

    fn write_all(
        &mut self,
        buffers: &[&[num_complex::Complex32]],
        _at_ns: Option<i64>,
        _end_burst: bool,
        _timeout_us: i64,
    ) -> Result<(), Error> {
        self.write(buffers)?;
        Ok(())
    }
}
