#![cfg_attr(not(test), no_std)]

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::i2c::I2c;

const READ_LEN: usize = 14;
pub const RAW_FRAME_LEN: usize = READ_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    TouchData,
    TouchPoints,
    TouchP1XHigh,
    TouchP1XLow,
    TouchP1YHigh,
    TouchP1YLow,
    TouchP2XHigh,
    TouchP2XLow,
    TouchP2YHigh,
    TouchP2YLow,
    Raw(u8),
}

impl Register {
    pub const fn addr(self) -> u8 {
        match self {
            Self::TouchData | Self::TouchPoints => 0x01,
            Self::TouchP1XHigh => 0x03,
            Self::TouchP1XLow => 0x04,
            Self::TouchP1YHigh => 0x05,
            Self::TouchP1YLow => 0x06,
            Self::TouchP2XHigh => 0x09,
            Self::TouchP2XLow => 0x0A,
            Self::TouchP2YHigh => 0x0B,
            Self::TouchP2YLow => 0x0C,
            Self::Raw(value) => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TouchSlot {
    First,
}

impl TouchSlot {
    const fn data_offset(self) -> usize {
        match self {
            Self::First => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TouchDataFrame {
    raw: [u8; READ_LEN],
}

impl TouchDataFrame {
    const fn new(raw: [u8; READ_LEN]) -> Self {
        Self { raw }
    }

    fn point_count_primary(&self) -> usize {
        (self.raw[1] & 0x0F).min(1) as usize
    }

    fn point(&self, slot: TouchSlot, config: &Axs5106Config) -> TouchPoint {
        let base = slot.data_offset();
        let raw_x = (u16::from(self.raw[base] & 0x0F) << 8) | u16::from(self.raw[base + 1]);
        let raw_y = (u16::from(self.raw[base + 2] & 0x0F) << 8) | u16::from(self.raw[base + 3]);
        let (x, y) = transform_point(config, raw_x, raw_y);

        TouchPoint {
            x,
            y,
            strength: None,
        }
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axs5106Config {
    pub address: u8,
    pub width: u16,
    pub height: u16,
    pub swap_xy: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub reset_active_high: bool,
}

impl Default for Axs5106Config {
    fn default() -> Self {
        Self {
            address: 0x63,
            width: 172,
            height: 320,
            swap_xy: false,
            mirror_x: false,
            mirror_y: false,
            reset_active_high: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
    pub strength: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<I2cError, PinError> {
    I2c(I2cError),
    Pin(PinError),
}

pub struct Axs5106<I2C, RST> {
    i2c: I2C,
    rst: Option<RST>,
    config: Axs5106Config,
}

#[cfg(feature = "async")]
pub struct Axs5106Async<I2C, RST, INT> {
    i2c: I2C,
    rst: Option<RST>,
    int: Option<INT>,
    config: Axs5106Config,
}

impl<I2C, RST> Axs5106<I2C, RST> {
    pub fn new(i2c: I2C, rst: Option<RST>, config: Axs5106Config) -> Self {
        Self { i2c, rst, config }
    }

    pub fn init(&mut self) -> Result<(), Error<I2C::Error, RST::Error>>
    where
        I2C: I2c,
        RST: OutputPin,
    {
        Ok(())
    }

    pub fn release(self) -> (I2C, Option<RST>) {
        (self.i2c, self.rst)
    }
}

#[cfg(feature = "async")]
impl<I2C, RST, INT> Axs5106Async<I2C, RST, INT> {
    pub fn new(i2c: I2C, rst: Option<RST>, int: Option<INT>, config: Axs5106Config) -> Self {
        Self {
            i2c,
            rst,
            int,
            config,
        }
    }

    pub fn init(&mut self) -> Result<(), Error<I2C::Error, RST::Error>>
    where
        I2C: embedded_hal_async::i2c::I2c,
        RST: OutputPin,
    {
        Ok(())
    }

    pub fn release(self) -> (I2C, Option<RST>, Option<INT>) {
        (self.i2c, self.rst, self.int)
    }
}

impl<I2C, RST, PinError> Axs5106<I2C, RST>
where
    I2C: I2c,
    RST: OutputPin<Error = PinError>,
{
    pub fn read_raw_frame(&mut self) -> Result<[u8; RAW_FRAME_LEN], Error<I2C::Error, PinError>> {
        self.read_frame()
    }

    fn read_frame(&mut self) -> Result<[u8; RAW_FRAME_LEN], Error<I2C::Error, PinError>> {
        let mut data = [0u8; READ_LEN];
        self.i2c
            .write(self.config.address, &[Register::TouchData.addr()])
            .map_err(Error::I2c)?;
        self.i2c
            .read(self.config.address, &mut data)
            .map_err(Error::I2c)?;
        Ok(data)
    }

    pub fn reset<D>(&mut self, delay: &mut D) -> Result<(), Error<I2C::Error, PinError>>
    where
        D: DelayNs,
    {
        if let Some(rst) = &mut self.rst {
            set_pin(rst, self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10);
            set_pin(rst, !self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10);
        }

        Ok(())
    }

    pub fn read_touches(
        &mut self,
    ) -> Result<Option<TouchPoint>, Error<I2C::Error, PinError>> {
        let data = self.read_frame()?;
        Ok(decode_touch_frame(TouchDataFrame::new(data), &self.config))
    }
}

#[cfg(feature = "async")]
impl<I2C, RST, INT, PinError> Axs5106Async<I2C, RST, INT>
where
    I2C: embedded_hal_async::i2c::I2c,
    RST: OutputPin<Error = PinError>,
    INT: embedded_hal_async::digital::Wait<Error = PinError>,
{
    pub async fn read_raw_frame(
        &mut self,
    ) -> Result<[u8; RAW_FRAME_LEN], Error<I2C::Error, PinError>> {
        self.read_frame().await
    }

    async fn read_frame(&mut self) -> Result<[u8; RAW_FRAME_LEN], Error<I2C::Error, PinError>> {
        let mut data = [0u8; READ_LEN];
        self.i2c
            .write(self.config.address, &[Register::TouchData.addr()])
            .await
            .map_err(Error::I2c)?;
        self.i2c
            .read(self.config.address, &mut data)
            .await
            .map_err(Error::I2c)?;
        Ok(data)
    }

    pub async fn reset<D>(&mut self, delay: &mut D) -> Result<(), Error<I2C::Error, PinError>>
    where
        D: embedded_hal_async::delay::DelayNs,
    {
        if let Some(rst) = &mut self.rst {
            set_pin(rst, self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10).await;
            set_pin(rst, !self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10).await;
        }

        Ok(())
    }

    pub async fn read_touches(
        &mut self,
    ) -> Result<Option<TouchPoint>, Error<I2C::Error, PinError>> {
        let data = self.read_frame().await?;
        Ok(decode_touch_frame(TouchDataFrame::new(data), &self.config))
    }

    pub async fn wait_for_touch<D>(
        &mut self,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error, PinError>>
    where
        D: embedded_hal_async::delay::DelayNs,
    {
        loop {
            if let Some(int) = &mut self.int {
                int.wait_for_any_edge().await.map_err(Error::Pin)?;
            } else {
                delay.delay_ms(10).await;
            }

            if self.read_touches().await?.is_some() {
                return Ok(());
            }
        }
    }
}

fn transform_point(config: &Axs5106Config, raw_x: u16, raw_y: u16) -> (u16, u16) {
    let (mut x, mut y) = if config.swap_xy {
        (raw_y, raw_x)
    } else {
        (raw_x, raw_y)
    };

    let (width, height) = if config.swap_xy {
        (config.height, config.width)
    } else {
        (config.width, config.height)
    };

    if config.mirror_x {
        x = width.saturating_sub(1).saturating_sub(x);
    }

    if config.mirror_y {
        y = height.saturating_sub(1).saturating_sub(y);
    }

    (x, y)
}

fn decode_touch_frame(
    frame: TouchDataFrame,
    config: &Axs5106Config,
) -> Option<TouchPoint> {
    let count = frame.point_count_primary().min(1);
    (count >= 1).then(|| frame.point(TouchSlot::First, config))
}

fn set_pin<P: OutputPin>(pin: &mut P, high: bool) -> Result<(), P::Error> {
    if high {
        pin.set_high()
    } else {
        pin.set_low()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::convert::Infallible;
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    use embedded_hal::digital::ErrorType as DigitalErrorType;
    use embedded_hal::i2c::{ErrorKind, ErrorType as I2cErrorType, Operation};
    use std::rc::Rc;
    use std::{boxed::Box, cell::RefCell, collections::VecDeque, vec::Vec};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct I2cMockError;

    impl embedded_hal::i2c::Error for I2cMockError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    #[derive(Clone, Default)]
    struct FakeI2c {
        responses: Rc<RefCell<VecDeque<Vec<u8>>>>,
    }

    impl FakeI2c {
        fn with_response(data: &[u8]) -> Self {
            let mut queue = VecDeque::new();
            queue.push_back(data.to_vec());
            Self {
                responses: Rc::new(RefCell::new(queue)),
            }
        }

        fn with_responses(responses: &[&[u8]]) -> Self {
            let mut queue = VecDeque::new();
            for response in responses {
                queue.push_back(response.to_vec());
            }
            Self {
                responses: Rc::new(RefCell::new(queue)),
            }
        }
    }

    impl I2cErrorType for FakeI2c {
        type Error = I2cMockError;
    }

    impl I2c for FakeI2c {
        fn transaction(
            &mut self,
            _address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            for operation in operations {
                match operation {
                    Operation::Read(buffer) => {
                        let response = self.responses.borrow_mut().pop_front().unwrap();
                        buffer.copy_from_slice(&response[..buffer.len()]);
                    }
                    Operation::Write(_) => {}
                }
            }
            Ok(())
        }
    }

    #[cfg(feature = "async")]
    impl embedded_hal_async::i2c::I2c for FakeI2c {
        async fn transaction(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            I2c::transaction(self, address, operations)
        }
    }

    #[derive(Clone, Default)]
    struct FakePin {
        states: Rc<RefCell<Vec<bool>>>,
    }

    impl FakePin {
        fn snapshot(&self) -> Vec<bool> {
            self.states.borrow().clone()
        }
    }

    impl DigitalErrorType for FakePin {
        type Error = Infallible;
    }

    impl OutputPin for FakePin {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.states.borrow_mut().push(false);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            self.states.borrow_mut().push(true);
            Ok(())
        }
    }

    #[cfg(feature = "async")]
    #[derive(Clone, Default)]
    struct FakeIntPin {
        waits: Rc<RefCell<usize>>,
    }

    #[cfg(feature = "async")]
    impl DigitalErrorType for FakeIntPin {
        type Error = Infallible;
    }

    #[cfg(feature = "async")]
    impl embedded_hal_async::digital::Wait for FakeIntPin {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            *self.waits.borrow_mut() += 1;
            Ok(())
        }

        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            *self.waits.borrow_mut() += 1;
            Ok(())
        }

        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            *self.waits.borrow_mut() += 1;
            Ok(())
        }

        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            *self.waits.borrow_mut() += 1;
            Ok(())
        }

        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            *self.waits.borrow_mut() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeDelay {
        calls: Vec<u32>,
    }

    impl DelayNs for FakeDelay {
        fn delay_ns(&mut self, ns: u32) {
            self.calls.push(ns.div_ceil(1_000_000));
        }

        fn delay_ms(&mut self, ms: u32) {
            self.calls.push(ms);
        }
    }

    #[cfg(feature = "async")]
    impl embedded_hal_async::delay::DelayNs for FakeDelay {
        async fn delay_ns(&mut self, ns: u32) {
            self.calls.push(ns.div_ceil(1_000_000));
        }

        async fn delay_ms(&mut self, ms: u32) {
            self.calls.push(ms);
        }
    }

    #[test]
    fn parses_zero_points() {
        let i2c = FakeI2c::with_response(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let rst = FakePin::default();
        let mut touch = Axs5106::new(i2c, Some(rst), Axs5106Config::default());

        let point = touch.read_touches().unwrap();

        assert_eq!(point, None);
    }

    #[test]
    fn truncates_two_points_to_single_point() {
        let data = [0, 2, 0x01, 0x23, 0x02, 0x34, 0, 0, 0x05, 0x67, 0x08, 0x9A, 0, 0];
        let i2c = FakeI2c::with_response(&data);
        let rst = FakePin::default();
        let mut touch = Axs5106::new(i2c, Some(rst), Axs5106Config::default());

        let point = touch.read_touches().unwrap();

        assert_eq!(point, Some(TouchPoint { x: 0x123, y: 0x234, strength: None }));
    }

    #[test]
    fn keeps_single_point_when_reported_count_is_one() {
        let data = [0, 1, 0x00, 0x20, 0x00, 0x30, 0, 0, 0x00, 0x50, 0x00, 0x60, 0, 0];
        let i2c = FakeI2c::with_response(&data);
        let rst = FakePin::default();
        let mut touch = Axs5106::new(i2c, Some(rst), Axs5106Config::default());

        let point = touch.read_touches().unwrap();

        assert_eq!(point, Some(TouchPoint { x: 0x20, y: 0x30, strength: None }));
    }

    #[test]
    fn applies_axis_transformations() {
        let data = [0, 1, 0x00, 0x02, 0x00, 0x03, 0, 0, 0, 0, 0, 0, 0, 0];
        let i2c = FakeI2c::with_response(&data);
        let rst = FakePin::default();
        let mut config = Axs5106Config::default();
        config.swap_xy = true;
        config.mirror_x = true;
        let mut touch = Axs5106::new(i2c, Some(rst), config);

        let point = touch.read_touches().unwrap().unwrap();

        assert_eq!(point.x, config.height - 1 - 3);
        assert_eq!(point.y, 2);
    }

    #[test]
    fn reset_toggles_reset_pin() {
        let i2c = FakeI2c::default();
        let rst = FakePin::default();
        let rst_probe = rst.clone();
        let mut touch = Axs5106::new(i2c, Some(rst), Axs5106Config::default());
        let mut delay = FakeDelay::default();

        touch.reset(&mut delay).unwrap();

        assert_eq!(rst_probe.snapshot(), vec![false, true]);
        assert_eq!(delay.calls, vec![10, 10]);
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_wait_for_touch_uses_interrupt_when_available() {
        let i2c = FakeI2c::with_response(&[0, 1, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
        let rst = FakePin::default();
        let int = FakeIntPin::default();
        let waits = int.waits.clone();
        let mut touch = Axs5106Async::new(i2c, Some(rst), Some(int), Axs5106Config::default());
        let mut delay = FakeDelay::default();

        block_on(async {
            touch.wait_for_touch(&mut delay).await.unwrap();
        });

        assert_eq!(*waits.borrow(), 1);
        assert!(delay.calls.is_empty());
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_wait_for_touch_polls_without_interrupt() {
        let i2c = FakeI2c::with_responses(&[
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0, 1, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0],
        ]);
        let rst = FakePin::default();
        let mut touch = Axs5106Async::new(i2c, Some(rst), Option::<FakeIntPin>::None, Axs5106Config::default());
        let mut delay = FakeDelay::default();

        block_on(async {
            touch.wait_for_touch(&mut delay).await.unwrap();
        });

        assert_eq!(delay.calls, vec![10, 10]);
    }

    #[cfg(feature = "async")]
    fn block_on<F: Future>(future: F) -> F::Output {
        fn noop_raw_waker() -> RawWaker {
            fn clone(_: *const ()) -> RawWaker {
                noop_raw_waker()
            }
            fn wake(_: *const ()) {}
            fn wake_by_ref(_: *const ()) {}
            fn drop(_: *const ()) {}

            RawWaker::new(
                core::ptr::null(),
                &RawWakerVTable::new(clone, wake, wake_by_ref, drop),
            )
        }

        let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
        let mut future = Box::pin(future);
        let mut context = Context::from_waker(&waker);

        loop {
            match Pin::as_mut(&mut future).poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => {}
            }
        }
    }
}
