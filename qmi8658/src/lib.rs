#![cfg_attr(not(test), no_std)]

use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::I2c;

const DATA_LEN: usize = 12;
const CHIP_ID: u8 = 0x05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    WhoAmI,
    Ctrl1,
    Ctrl2,
    Ctrl3,
    Ctrl7,
    Status0,
    AxL,
    Reset,
    Raw(u8),
}

impl Register {
    pub const fn addr(self) -> u8 {
        match self {
            Self::WhoAmI => 0x00,
            Self::Ctrl1 => 0x02,
            Self::Ctrl2 => 0x03,
            Self::Ctrl3 => 0x04,
            Self::Ctrl7 => 0x08,
            Self::Status0 => 0x2E,
            Self::AxL => 0x35,
            Self::Reset => 0x60,
            Self::Raw(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Qmi8658Config {
    pub address: u8,
    pub expected_chip_id: u8,
    pub ctrl1: u8,
    pub ctrl2: u8,
    pub ctrl3: u8,
    pub ctrl7: u8,
    pub reset_value: u8,
    pub reset_delay_ms: u32,
}

impl Default for Qmi8658Config {
    fn default() -> Self {
        Self {
            address: 0x6B,
            expected_chip_id: CHIP_ID,
            ctrl1: 0x40,
            ctrl2: 0x95,
            ctrl3: 0xD5,
            ctrl7: 0x03,
            reset_value: 0xB0,
            reset_delay_ms: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vec3i16 {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImuData {
    pub accel: Vec3i16,
    pub gyro: Vec3i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<I2cError> {
    I2c(I2cError),
    ChipIdMismatch { expected: u8, found: u8 },
}

pub struct Qmi8658<I2C> {
    i2c: I2C,
    config: Qmi8658Config,
}

#[cfg(feature = "async")]
pub struct Qmi8658Async<I2C> {
    i2c: I2C,
    config: Qmi8658Config,
}

impl<I2C> Qmi8658<I2C> {
    pub fn new(i2c: I2C, config: Qmi8658Config) -> Self {
        Self { i2c, config }
    }

    pub fn release(self) -> I2C {
        self.i2c
    }
}

#[cfg(feature = "async")]
impl<I2C> Qmi8658Async<I2C> {
    pub fn new(i2c: I2C, config: Qmi8658Config) -> Self {
        Self { i2c, config }
    }

    pub fn release(self) -> I2C {
        self.i2c
    }
}

impl<I2C> Qmi8658<I2C>
where
    I2C: I2c,
{
    pub fn init<D>(&mut self, delay: &mut D) -> Result<(), Error<I2C::Error>>
    where
        D: DelayNs,
    {
        let found = self.read_register(Register::WhoAmI)?;
        if found != self.config.expected_chip_id {
            return Err(Error::ChipIdMismatch {
                expected: self.config.expected_chip_id,
                found,
            });
        }

        self.write_register(Register::Reset, self.config.reset_value)?;
        delay.delay_ms(self.config.reset_delay_ms);
        self.write_register(Register::Ctrl1, self.config.ctrl1)?;
        self.write_register(Register::Ctrl7, self.config.ctrl7)?;
        self.write_register(Register::Ctrl2, self.config.ctrl2)?;
        self.write_register(Register::Ctrl3, self.config.ctrl3)?;

        Ok(())
    }

    pub fn read_chip_id(&mut self) -> Result<u8, Error<I2C::Error>> {
        self.read_register(Register::WhoAmI)
    }

    pub fn read_status(&mut self) -> Result<u8, Error<I2C::Error>> {
        self.read_register(Register::Status0)
    }

    pub fn read_data(&mut self) -> Result<ImuData, Error<I2C::Error>> {
        let mut raw = [0u8; DATA_LEN];
        self.read_registers(Register::AxL, &mut raw)?;
        Ok(parse_data(raw))
    }

    pub fn read_data_if_ready(&mut self) -> Result<Option<ImuData>, Error<I2C::Error>> {
        let status = self.read_status()?;
        if status & 0x03 == 0 {
            return Ok(None);
        }
        self.read_data().map(Some)
    }

    fn read_register(&mut self, reg: Register) -> Result<u8, Error<I2C::Error>> {
        let mut value = [0u8; 1];
        self.read_registers(reg, &mut value)?;
        Ok(value[0])
    }

    fn read_registers(&mut self, reg: Register, buf: &mut [u8]) -> Result<(), Error<I2C::Error>> {
        self.i2c
            .write_read(self.config.address, &[reg.addr()], buf)
            .map_err(Error::I2c)
    }

    fn write_register(&mut self, reg: Register, value: u8) -> Result<(), Error<I2C::Error>> {
        self.i2c
            .write(self.config.address, &[reg.addr(), value])
            .map_err(Error::I2c)
    }
}

#[cfg(feature = "async")]
impl<I2C> Qmi8658Async<I2C>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    pub async fn init<D>(&mut self, delay: &mut D) -> Result<(), Error<I2C::Error>>
    where
        D: embedded_hal_async::delay::DelayNs,
    {
        let found = self.read_register(Register::WhoAmI).await?;
        if found != self.config.expected_chip_id {
            return Err(Error::ChipIdMismatch {
                expected: self.config.expected_chip_id,
                found,
            });
        }

        self.write_register(Register::Reset, self.config.reset_value)
            .await?;
        delay.delay_ms(self.config.reset_delay_ms).await;
        self.write_register(Register::Ctrl1, self.config.ctrl1).await?;
        self.write_register(Register::Ctrl7, self.config.ctrl7).await?;
        self.write_register(Register::Ctrl2, self.config.ctrl2).await?;
        self.write_register(Register::Ctrl3, self.config.ctrl3).await?;

        Ok(())
    }

    pub async fn read_chip_id(&mut self) -> Result<u8, Error<I2C::Error>> {
        self.read_register(Register::WhoAmI).await
    }

    pub async fn read_status(&mut self) -> Result<u8, Error<I2C::Error>> {
        self.read_register(Register::Status0).await
    }

    pub async fn read_data(&mut self) -> Result<ImuData, Error<I2C::Error>> {
        let mut raw = [0u8; DATA_LEN];
        self.read_registers(Register::AxL, &mut raw).await?;
        Ok(parse_data(raw))
    }

    pub async fn read_data_if_ready(&mut self) -> Result<Option<ImuData>, Error<I2C::Error>> {
        let status = self.read_status().await?;
        if status & 0x03 == 0 {
            return Ok(None);
        }
        self.read_data().await.map(Some)
    }

    async fn read_register(&mut self, reg: Register) -> Result<u8, Error<I2C::Error>> {
        let mut value = [0u8; 1];
        self.read_registers(reg, &mut value).await?;
        Ok(value[0])
    }

    async fn read_registers(
        &mut self,
        reg: Register,
        buf: &mut [u8],
    ) -> Result<(), Error<I2C::Error>> {
        self.i2c
            .write_read(self.config.address, &[reg.addr()], buf)
            .await
            .map_err(Error::I2c)
    }

    async fn write_register(&mut self, reg: Register, value: u8) -> Result<(), Error<I2C::Error>> {
        self.i2c
            .write(self.config.address, &[reg.addr(), value])
            .await
            .map_err(Error::I2c)
    }
}

fn parse_data(raw: [u8; DATA_LEN]) -> ImuData {
    ImuData {
        accel: Vec3i16 {
            x: i16::from_le_bytes([raw[0], raw[1]]),
            y: i16::from_le_bytes([raw[2], raw[3]]),
            z: i16::from_le_bytes([raw[4], raw[5]]),
        },
        gyro: Vec3i16 {
            x: i16::from_le_bytes([raw[6], raw[7]]),
            y: i16::from_le_bytes([raw[8], raw[9]]),
            z: i16::from_le_bytes([raw[10], raw[11]]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::i2c::{ErrorKind, ErrorType, I2c, Operation};
    use std::{cell::RefCell, collections::VecDeque, rc::Rc, vec::Vec};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct I2cMockError;

    impl embedded_hal::i2c::Error for I2cMockError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    #[derive(Clone, Default)]
    struct FakeI2c {
        reads: Rc<RefCell<VecDeque<Vec<u8>>>>,
        writes: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    impl FakeI2c {
        fn with_reads(responses: &[&[u8]]) -> Self {
            let mut reads = VecDeque::new();
            for response in responses {
                reads.push_back(response.to_vec());
            }
            Self {
                reads: Rc::new(RefCell::new(reads)),
                writes: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl ErrorType for FakeI2c {
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
                        let data = self.reads.borrow_mut().pop_front().unwrap();
                        buffer.copy_from_slice(&data[..buffer.len()]);
                    }
                    Operation::Write(data) => {
                        self.writes.borrow_mut().push(data.to_vec());
                    }
                }
            }
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

    #[test]
    fn init_writes_expected_sequence() {
        let i2c = FakeI2c::with_reads(&[&[CHIP_ID]]);
        let writes = i2c.writes.clone();
        let mut imu = Qmi8658::new(i2c, Qmi8658Config::default());
        let mut delay = FakeDelay::default();

        imu.init(&mut delay).unwrap();

        assert_eq!(delay.calls, vec![10]);
        let writes = writes.borrow();
        assert_eq!(writes[0], vec![Register::WhoAmI.addr()]);
        assert_eq!(writes[1], vec![Register::Reset.addr(), 0xB0]);
        assert_eq!(writes[2], vec![Register::Ctrl1.addr(), 0x40]);
        assert_eq!(writes[3], vec![Register::Ctrl7.addr(), 0x03]);
        assert_eq!(writes[4], vec![Register::Ctrl2.addr(), 0x95]);
        assert_eq!(writes[5], vec![Register::Ctrl3.addr(), 0xD5]);
    }

    #[test]
    fn read_if_ready_returns_none_when_no_data() {
        let i2c = FakeI2c::with_reads(&[&[0x00]]);
        let mut imu = Qmi8658::new(i2c, Qmi8658Config::default());

        let result = imu.read_data_if_ready().unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn read_parses_accel_and_gyro() {
        let i2c = FakeI2c::with_reads(&[&[
            0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A, 0x10, 0x00, 0x20, 0x00, 0x30, 0x00,
        ]]);
        let mut imu = Qmi8658::new(i2c, Qmi8658Config::default());

        let data = imu.read_data().unwrap();

        assert_eq!(data.accel.x, 0x1234);
        assert_eq!(data.accel.y, 0x5678);
        assert_eq!(data.accel.z, -25924);
        assert_eq!(data.gyro.x, 16);
        assert_eq!(data.gyro.y, 32);
        assert_eq!(data.gyro.z, 48);
    }

    #[test]
    fn rejects_wrong_chip_id() {
        let i2c = FakeI2c::with_reads(&[&[0x00]]);
        let mut imu = Qmi8658::new(i2c, Qmi8658Config::default());
        let mut delay = FakeDelay::default();

        let result = imu.init(&mut delay);

        assert_eq!(
            result,
            Err(Error::ChipIdMismatch {
                expected: CHIP_ID,
                found: 0x00
            })
        );
    }
}
