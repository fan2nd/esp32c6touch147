#![cfg_attr(not(test), no_std)]

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::SpiDevice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SoftwareReset,
    SleepOut,
    InversionOff,
    InversionOn,
    DisplayOff,
    DisplayOn,
    ColumnAddressSet,
    RowAddressSet,
    MemoryWrite,
    MemoryAccessControl,
    InterfacePixelFormat,
    Raw(u8),
}

impl Command {
    pub const fn code(self) -> u8 {
        match self {
            Self::SoftwareReset => 0x01,
            Self::SleepOut => 0x11,
            Self::InversionOff => 0x20,
            Self::InversionOn => 0x21,
            Self::DisplayOff => 0x28,
            Self::DisplayOn => 0x29,
            Self::ColumnAddressSet => 0x2A,
            Self::RowAddressSet => 0x2B,
            Self::MemoryWrite => 0x2C,
            Self::MemoryAccessControl => 0x36,
            Self::InterfacePixelFormat => 0x3A,
            Self::Raw(value) => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryAccessControl(u8);

impl MemoryAccessControl {
    const MIRROR_Y: u8 = 0x80;
    const MIRROR_X: u8 = 0x40;
    const SWAP_XY: u8 = 0x20;
    const BGR_ORDER: u8 = 0x08;

    const fn from_parts(orientation: Orientation, color_order: ColorOrder) -> Self {
        let orientation_bits = match orientation {
            Orientation::Portrait => 0,
            Orientation::Landscape => Self::SWAP_XY | Self::MIRROR_X,
            Orientation::PortraitFlipped => Self::MIRROR_X | Self::MIRROR_Y,
            Orientation::LandscapeFlipped => Self::SWAP_XY | Self::MIRROR_Y,
        };

        let color_bits = match color_order {
            ColorOrder::Rgb => 0,
            ColorOrder::Bgr => Self::BGR_ORDER,
        };

        Self(orientation_bits | color_bits)
    }

    const fn into_byte(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InterfacePixelFormat(u8);

impl InterfacePixelFormat {
    const fn from_pixel_format(pixel_format: PixelFormat) -> Self {
        match pixel_format {
            PixelFormat::Rgb565 => Self(0x55),
            PixelFormat::Rgb666 => Self(0x66),
        }
    }

    const fn into_byte(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitCommand<'a> {
    pub cmd: Command,
    pub data: &'a [u8],
    pub delay_ms: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorOrder {
    Rgb,
    Bgr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb565,
    Rgb666,
}

impl PixelFormat {
    const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb565 => 2,
            Self::Rgb666 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
    PortraitFlipped,
    LandscapeFlipped,
}

impl Orientation {
    const fn swaps_axes(self) -> bool {
        matches!(self, Self::Landscape | Self::LandscapeFlipped)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Jd9853Config<'a> {
    pub width: u16,
    pub height: u16,
    pub x_offset: u16,
    pub y_offset: u16,
    pub color_order: ColorOrder,
    pub pixel_format: PixelFormat,
    pub reset_active_high: bool,
    pub invert_colors: bool,
    pub orientation: Orientation,
    pub init_commands: &'a [InitCommand<'a>],
}

impl Default for Jd9853Config<'static> {
    fn default() -> Self {
        Self {
            width: 172,
            height: 320,
            x_offset: 34,
            y_offset: 0,
            color_order: ColorOrder::Rgb,
            pixel_format: PixelFormat::Rgb565,
            reset_active_high: true,
            invert_colors: false,
            orientation: Orientation::Portrait,
            init_commands: &DEFAULT_INIT_COMMANDS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<SpiError, PinError> {
    Spi(SpiError),
    Pin(PinError),
    InvalidInput,
    UnsupportedPixelFormat,
}

pub struct Jd9853<'a, SPI, DC, RST> {
    spi: SPI,
    dc: DC,
    rst: Option<RST>,
    config: Jd9853Config<'a>,
    madctl: MemoryAccessControl,
    colmod: InterfacePixelFormat,
}

#[cfg(feature = "async")]
pub struct Jd9853Async<'a, SPI, DC, RST> {
    spi: SPI,
    dc: DC,
    rst: Option<RST>,
    config: Jd9853Config<'a>,
    madctl: MemoryAccessControl,
    colmod: InterfacePixelFormat,
}

impl<'a, SPI, DC, RST> Jd9853<'a, SPI, DC, RST> {
    pub fn new(spi: SPI, dc: DC, rst: Option<RST>, config: Jd9853Config<'a>) -> Self {
        let madctl = MemoryAccessControl::from_parts(config.orientation, config.color_order);
        let colmod = InterfacePixelFormat::from_pixel_format(config.pixel_format);

        Self {
            spi,
            dc,
            rst,
            config,
            madctl,
            colmod,
        }
    }

    pub fn release(self) -> (SPI, DC, Option<RST>) {
        (self.spi, self.dc, self.rst)
    }

    pub fn size(&self) -> (u16, u16) {
        if self.config.orientation.swaps_axes() {
            (self.config.height, self.config.width)
        } else {
            (self.config.width, self.config.height)
        }
    }
}

#[cfg(feature = "async")]
impl<'a, SPI, DC, RST> Jd9853Async<'a, SPI, DC, RST> {
    pub fn new(spi: SPI, dc: DC, rst: Option<RST>, config: Jd9853Config<'a>) -> Self {
        let madctl = MemoryAccessControl::from_parts(config.orientation, config.color_order);
        let colmod = InterfacePixelFormat::from_pixel_format(config.pixel_format);

        Self {
            spi,
            dc,
            rst,
            config,
            madctl,
            colmod,
        }
    }

    pub fn release(self) -> (SPI, DC, Option<RST>) {
        (self.spi, self.dc, self.rst)
    }

    pub fn size(&self) -> (u16, u16) {
        if self.config.orientation.swaps_axes() {
            (self.config.height, self.config.width)
        } else {
            (self.config.width, self.config.height)
        }
    }
}

impl<'a, SPI, DC, RST, PinError> Jd9853<'a, SPI, DC, RST>
where
    SPI: SpiDevice<u8>,
    DC: OutputPin<Error = PinError>,
    RST: OutputPin<Error = PinError>,
{
    pub fn reset<D>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error, PinError>>
    where
        D: DelayNs,
    {
        if let Some(rst) = &mut self.rst {
            set_pin(rst, self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10);
            set_pin(rst, !self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10);
        } else {
            self.write_command(Command::SoftwareReset)?;
            delay.delay_ms(20);
        }

        Ok(())
    }

    pub fn init<D>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error, PinError>>
    where
        D: DelayNs,
    {
        self.write_command(Command::SleepOut)?;
        delay.delay_ms(100);

        self.write_command_data(
            Command::MemoryAccessControl,
            &[self.madctl.into_byte()],
        )?;
        self.write_command_data(
            Command::InterfacePixelFormat,
            &[self.colmod.into_byte()],
        )?;

        for command in self.config.init_commands {
            if command.cmd == Command::MemoryAccessControl && !command.data.is_empty() {
                self.madctl = MemoryAccessControl(command.data[0]);
            } else if command.cmd == Command::InterfacePixelFormat && !command.data.is_empty() {
                self.colmod = InterfacePixelFormat(command.data[0]);
            }

            self.write_command_data(command.cmd, command.data)?;
            if command.delay_ms > 0 {
                delay.delay_ms(u32::from(command.delay_ms));
            }
        }

        if self.config.invert_colors {
            self.set_invert(true)?;
        }

        Ok(())
    }

    pub fn set_orientation(
        &mut self,
        orientation: Orientation,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.config.orientation = orientation;
        self.madctl = MemoryAccessControl::from_parts(orientation, self.config.color_order);
        self.write_command_data(Command::MemoryAccessControl, &[self.madctl.into_byte()])
    }

    pub fn set_invert(&mut self, invert: bool) -> Result<(), Error<SPI::Error, PinError>> {
        self.config.invert_colors = invert;
        self.write_command(if invert {
            Command::InversionOn
        } else {
            Command::InversionOff
        })
    }

    pub fn set_display_on(&mut self, on: bool) -> Result<(), Error<SPI::Error, PinError>> {
        self.write_command(if on {
            Command::DisplayOn
        } else {
            Command::DisplayOff
        })
    }

    pub fn set_address_window(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        let (max_width, max_height) = self.size();
        if width == 0
            || height == 0
            || x >= max_width
            || y >= max_height
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
            || x + width > max_width
            || y + height > max_height
        {
            return Err(Error::InvalidInput);
        }

        let x_start = x + self.config.x_offset;
        let x_end = x_start + width - 1;
        let y_start = y + self.config.y_offset;
        let y_end = y_start + height - 1;

        self.write_command_data(Command::ColumnAddressSet, &pack_range(x_start, x_end))?;
        self.write_command_data(Command::RowAddressSet, &pack_range(y_start, y_end))?;
        Ok(())
    }

    pub fn write_pixels(&mut self, pixels: &[u8]) -> Result<(), Error<SPI::Error, PinError>> {
        let bytes_per_pixel = self.config.pixel_format.bytes_per_pixel();
        if pixels.is_empty() || pixels.len() % bytes_per_pixel != 0 {
            return Err(Error::InvalidInput);
        }

        self.write_command(Command::MemoryWrite)?;
        self.write_data(pixels)
    }

    pub fn write_command(&mut self, command: Command) -> Result<(), Error<SPI::Error, PinError>> {
        self.dc.set_low().map_err(Error::Pin)?;
        self.spi.write(&[command.code()]).map_err(Error::Spi)
    }

    pub fn write_command_data(
        &mut self,
        command: Command,
        data: &[u8],
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.write_command(command)?;
        if !data.is_empty() {
            self.write_data(data)?;
        }
        Ok(())
    }

    pub fn write_data(&mut self, data: &[u8]) -> Result<(), Error<SPI::Error, PinError>> {
        if data.is_empty() {
            return Ok(());
        }

        self.dc.set_high().map_err(Error::Pin)?;
        self.spi.write(data).map_err(Error::Spi)
    }
}

#[cfg(feature = "async")]
impl<'a, SPI, DC, RST, PinError> Jd9853Async<'a, SPI, DC, RST>
where
    SPI: embedded_hal_async::spi::SpiDevice<u8>,
    DC: OutputPin<Error = PinError>,
    RST: OutputPin<Error = PinError>,
{
    pub async fn reset<D>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error, PinError>>
    where
        D: embedded_hal_async::delay::DelayNs,
    {
        if let Some(rst) = &mut self.rst {
            set_pin(rst, self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10).await;
            set_pin(rst, !self.config.reset_active_high).map_err(Error::Pin)?;
            delay.delay_ms(10).await;
        } else {
            self.write_command(Command::SoftwareReset).await?;
            delay.delay_ms(20).await;
        }

        Ok(())
    }

    pub async fn init<D>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error, PinError>>
    where
        D: embedded_hal_async::delay::DelayNs,
    {
        self.write_command(Command::SleepOut).await?;
        delay.delay_ms(100).await;

        self.write_command_data(
            Command::MemoryAccessControl,
            &[self.madctl.into_byte()],
        )
            .await?;
        self.write_command_data(
            Command::InterfacePixelFormat,
            &[self.colmod.into_byte()],
        )
            .await?;

        for command in self.config.init_commands {
            if command.cmd == Command::MemoryAccessControl && !command.data.is_empty() {
                self.madctl = MemoryAccessControl(command.data[0]);
            } else if command.cmd == Command::InterfacePixelFormat && !command.data.is_empty() {
                self.colmod = InterfacePixelFormat(command.data[0]);
            }

            self.write_command_data(command.cmd, command.data).await?;
            if command.delay_ms > 0 {
                delay.delay_ms(u32::from(command.delay_ms)).await;
            }
        }

        if self.config.invert_colors {
            self.set_invert(true).await?;
        }

        Ok(())
    }

    pub async fn set_orientation(
        &mut self,
        orientation: Orientation,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.config.orientation = orientation;
        self.madctl = MemoryAccessControl::from_parts(orientation, self.config.color_order);
        self.write_command_data(Command::MemoryAccessControl, &[self.madctl.into_byte()])
            .await
    }

    pub async fn set_invert(
        &mut self,
        invert: bool,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.config.invert_colors = invert;
        self.write_command(if invert {
            Command::InversionOn
        } else {
            Command::InversionOff
        })
        .await
    }

    pub async fn set_display_on(
        &mut self,
        on: bool,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.write_command(if on {
            Command::DisplayOn
        } else {
            Command::DisplayOff
        })
        .await
    }

    pub async fn set_address_window(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        let (max_width, max_height) = self.size();
        if width == 0
            || height == 0
            || x >= max_width
            || y >= max_height
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
            || x + width > max_width
            || y + height > max_height
        {
            return Err(Error::InvalidInput);
        }

        let x_start = x + self.config.x_offset;
        let x_end = x_start + width - 1;
        let y_start = y + self.config.y_offset;
        let y_end = y_start + height - 1;

        self.write_command_data(Command::ColumnAddressSet, &pack_range(x_start, x_end))
            .await?;
        self.write_command_data(Command::RowAddressSet, &pack_range(y_start, y_end))
            .await?;
        Ok(())
    }

    pub async fn write_pixels(
        &mut self,
        pixels: &[u8],
    ) -> Result<(), Error<SPI::Error, PinError>> {
        let bytes_per_pixel = self.config.pixel_format.bytes_per_pixel();
        if pixels.is_empty() || pixels.len() % bytes_per_pixel != 0 {
            return Err(Error::InvalidInput);
        }

        self.write_command(Command::MemoryWrite).await?;
        self.write_data(pixels).await
    }

    pub async fn write_command(
        &mut self,
        command: Command,
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.dc.set_low().map_err(Error::Pin)?;
        self.spi.write(&[command.code()]).await.map_err(Error::Spi)
    }

    pub async fn write_command_data(
        &mut self,
        command: Command,
        data: &[u8],
    ) -> Result<(), Error<SPI::Error, PinError>> {
        self.write_command(command).await?;
        if !data.is_empty() {
            self.write_data(data).await?;
        }
        Ok(())
    }

    pub async fn write_data(&mut self, data: &[u8]) -> Result<(), Error<SPI::Error, PinError>> {
        if data.is_empty() {
            return Ok(());
        }

        self.dc.set_high().map_err(Error::Pin)?;
        self.spi.write(data).await.map_err(Error::Spi)
    }
}

fn pack_range(start: u16, end: u16) -> [u8; 4] {
    [
        (start >> 8) as u8,
        start as u8,
        (end >> 8) as u8,
        end as u8,
    ]
}

fn set_pin<P: OutputPin>(pin: &mut P, high: bool) -> Result<(), P::Error> {
    if high {
        pin.set_high()
    } else {
        pin.set_low()
    }
}

const DEFAULT_INIT_COMMANDS: [InitCommand<'static>; 32] = [
    InitCommand {
        cmd: Command::SleepOut,
        data: &[],
        delay_ms: 120,
    },
    InitCommand {
        cmd: Command::Raw(0xDF),
        data: &[0x98, 0x53],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDF),
        data: &[0x98, 0x53],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xB2),
        data: &[0x23],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xB7),
        data: &[0x00, 0x47, 0x00, 0x6F],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xBB),
        data: &[0x1C, 0x1A, 0x55, 0x73, 0x63, 0xF0],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC0),
        data: &[0x44, 0xA4],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC1),
        data: &[0x16],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC3),
        data: &[0x7D, 0x07, 0x14, 0x06, 0xCF, 0x71, 0x72, 0x77],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC4),
        data: &[0x00, 0x00, 0xA0, 0x79, 0x0B, 0x0A, 0x16, 0x79, 0x0B, 0x0A, 0x16, 0x82],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC8),
        data: &[
            0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12,
            0x0D, 0x04, 0x00, 0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26,
            0x25, 0x17, 0x12, 0x0D, 0x04, 0x00,
        ],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xD0),
        data: &[0x04, 0x06, 0x6B, 0x0F, 0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xD7),
        data: &[0x00, 0x30],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xE6),
        data: &[0x14],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDE),
        data: &[0x01],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xB7),
        data: &[0x03, 0x13, 0xEF, 0x35, 0x35],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC1),
        data: &[0x14, 0x15, 0xC0],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC2),
        data: &[0x06, 0x3A],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xC4),
        data: &[0x72, 0x12],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xBE),
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDE),
        data: &[0x02],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xE5),
        data: &[0x00, 0x02, 0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xE5),
        data: &[0x01, 0x02, 0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDE),
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0x35),
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::InterfacePixelFormat,
        data: &[0x05],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::ColumnAddressSet,
        data: &[0x00, 0x22, 0x00, 0xCD],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::RowAddressSet,
        data: &[0x00, 0x00, 0x01, 0x3F],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDE),
        data: &[0x02],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xE5),
        data: &[0x00, 0x02, 0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::Raw(0xDE),
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        cmd: Command::DisplayOn,
        data: &[],
        delay_ms: 0,
    },
];

#[cfg(feature = "graphics")]
mod graphics_support {
    use super::{Command, Error, Jd9853, PixelFormat};
    use embedded_graphics_core::{
        Pixel,
        draw_target::DrawTarget,
        geometry::{Dimensions, OriginDimensions, Size},
        pixelcolor::{IntoStorage, Rgb565},
        prelude::Point,
        primitives::Rectangle,
    };
    use embedded_hal::digital::OutputPin;
    use embedded_hal::spi::SpiDevice;

    impl<'a, SPI, DC, RST, PinError> OriginDimensions for Jd9853<'a, SPI, DC, RST>
    where
        SPI: SpiDevice<u8>,
        DC: OutputPin<Error = PinError>,
        RST: OutputPin<Error = PinError>,
    {
        fn size(&self) -> Size {
            let (width, height) = Jd9853::size(self);
            Size::new(width as u32, height as u32)
        }
    }

    impl<'a, SPI, DC, RST, PinError> DrawTarget for Jd9853<'a, SPI, DC, RST>
    where
        SPI: SpiDevice<u8>,
        DC: OutputPin<Error = PinError>,
        RST: OutputPin<Error = PinError>,
    {
        type Color = Rgb565;
        type Error = Error<SPI::Error, PinError>;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = Pixel<Self::Color>>,
        {
            if self.config.pixel_format != PixelFormat::Rgb565 {
                return Err(Error::UnsupportedPixelFormat);
            }

            let bounding_box = self.bounding_box();
            for Pixel(point, color) in pixels {
                if !bounding_box.contains(point) {
                    continue;
                }

                let x = point.x as u16;
                let y = point.y as u16;
                self.set_address_window(x, y, 1, 1)?;
                self.write_pixels(&color.into_storage().to_be_bytes())?;
            }

            Ok(())
        }

        fn fill_solid(
            &mut self,
            area: &Rectangle,
            color: Self::Color,
        ) -> Result<(), Self::Error> {
            if self.config.pixel_format != PixelFormat::Rgb565 {
                return Err(Error::UnsupportedPixelFormat);
            }

            let area = area.intersection(&self.bounding_box());
            let Some(_bottom_right) = area.bottom_right() else {
                return Ok(());
            };

            let width = area.size.width as u16;
            let height = area.size.height as u16;
            self.set_address_window(area.top_left.x as u16, area.top_left.y as u16, width, height)?;

            let pixel = color.into_storage().to_be_bytes();
            let mut buffer = [0u8; 128];
            for chunk in buffer.chunks_exact_mut(2) {
                chunk.copy_from_slice(&pixel);
            }

            let mut remaining_pixels = usize::from(width) * usize::from(height);
            self.write_command(Command::MemoryWrite)?;
            while remaining_pixels > 0 {
                let chunk_pixels = remaining_pixels.min(buffer.len() / 2);
                self.write_data(&buffer[..chunk_pixels * 2])?;
                remaining_pixels -= chunk_pixels;
            }

            Ok(())
        }

        fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
            self.fill_solid(
                &Rectangle::new(Point::zero(), <Self as OriginDimensions>::size(self)),
                color,
            )
        }
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
    use embedded_hal::spi::{ErrorKind, ErrorType as SpiErrorType, Operation};
    use std::rc::Rc;
    use std::{boxed::Box, cell::RefCell, vec::Vec};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct SpiMockError;

    impl embedded_hal::spi::Error for SpiMockError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    #[derive(Clone, Default)]
    struct FakeSpi {
        writes: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    impl FakeSpi {
        fn snapshot(&self) -> Vec<Vec<u8>> {
            self.writes.borrow().clone()
        }
    }

    impl SpiErrorType for FakeSpi {
        type Error = SpiMockError;
    }

    impl SpiDevice<u8> for FakeSpi {
        fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Self::Error> {
            for operation in operations {
                match operation {
                    Operation::Write(data) => self.writes.borrow_mut().push(data.to_vec()),
                    Operation::Transfer(_, _)
                    | Operation::TransferInPlace(_)
                    | Operation::Read(_)
                    | Operation::DelayNs(_) => {}
                }
            }
            Ok(())
        }
    }

    #[cfg(feature = "async")]
    impl embedded_hal_async::spi::SpiDevice<u8> for FakeSpi {
        async fn transaction(
            &mut self,
            operations: &mut [Operation<'_, u8>],
        ) -> Result<(), Self::Error> {
            SpiDevice::transaction(self, operations)
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
    fn reset_uses_hardware_pin_when_available() {
        let spi = FakeSpi::default();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let rst_probe = rst.clone();
        let mut display = Jd9853::new(spi, dc, Some(rst), Jd9853Config::default());
        let mut delay = FakeDelay::default();

        display.reset(&mut delay).unwrap();

        assert_eq!(rst_probe.snapshot(), vec![true, false]);
        assert_eq!(delay.calls, vec![10, 10]);
    }

    #[test]
    fn init_sends_expected_prefix_and_delays() {
        let spi = FakeSpi::default();
        let spi_probe = spi.clone();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let mut display = Jd9853::new(spi, dc, Some(rst), Jd9853Config::default());
        let mut delay = FakeDelay::default();

        display.init(&mut delay).unwrap();

        let writes = spi_probe.snapshot();
        assert_eq!(writes[0], vec![Command::SleepOut.code()]);
        assert_eq!(writes[1], vec![Command::MemoryAccessControl.code()]);
        assert_eq!(
            writes[2],
            vec![MemoryAccessControl::from_parts(Orientation::Portrait, ColorOrder::Rgb)
                .into_byte()]
        );
        assert_eq!(writes[3], vec![Command::InterfacePixelFormat.code()]);
        assert_eq!(
            writes[4],
            vec![InterfacePixelFormat::from_pixel_format(PixelFormat::Rgb565).into_byte()]
        );
        assert_eq!(delay.calls[0], 100);
        assert!(delay.calls.contains(&120));
    }

    #[test]
    fn set_address_window_applies_offsets() {
        let spi = FakeSpi::default();
        let spi_probe = spi.clone();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let mut display = Jd9853::new(spi, dc, Some(rst), Jd9853Config::default());

        display.set_address_window(0, 0, 2, 3).unwrap();

        let writes = spi_probe.snapshot();
        assert_eq!(writes[0], vec![Command::ColumnAddressSet.code()]);
        assert_eq!(writes[1], vec![0x00, 0x22, 0x00, 0x23]);
        assert_eq!(writes[2], vec![Command::RowAddressSet.code()]);
        assert_eq!(writes[3], vec![0x00, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn orientation_updates_madctl() {
        let spi = FakeSpi::default();
        let spi_probe = spi.clone();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let mut display = Jd9853::new(spi, dc, Some(rst), Jd9853Config::default());

        display.set_orientation(Orientation::Landscape).unwrap();

        let writes = spi_probe.snapshot();
        assert_eq!(writes[0], vec![Command::MemoryAccessControl.code()]);
        assert_eq!(
            writes[1],
            vec![MemoryAccessControl::from_parts(Orientation::Landscape, ColorOrder::Rgb)
                .into_byte()]
        );
        assert_eq!(display.size(), (320, 172));
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn fill_solid_streams_rgb565_pixels() {
        use embedded_graphics_core::{
            draw_target::DrawTarget,
            geometry::Size,
            pixelcolor::{Rgb565, RgbColor},
            prelude::Point,
            primitives::Rectangle,
        };

        let spi = FakeSpi::default();
        let spi_probe = spi.clone();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let mut display = Jd9853::new(spi, dc, Some(rst), Jd9853Config::default());

        display
            .fill_solid(&Rectangle::new(Point::new(0, 0), Size::new(4, 2)), Rgb565::RED)
            .unwrap();

        let writes = spi_probe.snapshot();
        assert_eq!(writes[4], vec![Command::MemoryWrite.code()]);
        assert_eq!(writes[5].len(), 16);
        assert_eq!(writes.len(), 6);
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_write_pixels_works() {
        let spi = FakeSpi::default();
        let spi_probe = spi.clone();
        let dc = FakePin::default();
        let rst = FakePin::default();
        let mut display = Jd9853Async::new(spi, dc, Some(rst), Jd9853Config::default());

        block_on(async {
            display.set_address_window(1, 2, 1, 1).await.unwrap();
            display.write_pixels(&[0x12, 0x34]).await.unwrap();
        });

        let writes = spi_probe.snapshot();
        assert_eq!(
            writes[writes.len() - 2],
            vec![Command::MemoryWrite.code()]
        );
        assert_eq!(writes[writes.len() - 1], vec![0x12, 0x34]);
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
