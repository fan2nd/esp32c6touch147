#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    Pixel,
    mono_font::{
        MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle, RoundedRectangle},
    text::Text,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use jd9853::{Jd9853, Jd9853Config};
use log::info;
use u8g2_fonts::{
    Error as U8g2Error, FontRenderer, fonts,
    types::{FontColor, VerticalPosition},
};

use esp_backtrace as _;

const SCREEN_WIDTH: i32 = 172;
const HEADER_HEIGHT: i32 = 56;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const CARD_FILL: Rgb565 = Rgb565::new(4, 12, 18);
const CARD_BORDER: Rgb565 = Rgb565::new(8, 20, 30);
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const LABEL_COLOR: Rgb565 = Rgb565::new(18, 36, 44);
const SAMPLE_COLOR: Rgb565 = Rgb565::new(31, 56, 20);
const DEMO_TEXT: &str = "rust, 牛逼";
const BASE_FONT: FontRenderer = FontRenderer::new::<fonts::u8g2_font_wqy16_t_gb2312>();

struct ScaledDrawTarget<'a, DRAW> {
    inner: &'a mut DRAW,
    offset: Point,
    scale_num: i32,
    scale_den: i32,
}

impl<'a, DRAW> ScaledDrawTarget<'a, DRAW> {
    fn new(inner: &'a mut DRAW, offset: Point, scale_num: i32, scale_den: i32) -> Self {
        Self {
            inner,
            offset,
            scale_num,
            scale_den,
        }
    }

    fn scale_coord(&self, value: i32) -> i32 {
        (value * self.scale_num) / self.scale_den
    }

    fn scale_span(&self, start: i32, end: i32) -> u32 {
        (self.scale_coord(end) - self.scale_coord(start)) as u32
    }
}

impl<DRAW> DrawTarget for ScaledDrawTarget<'_, DRAW>
where
    DRAW: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    type Color = Rgb565;
    type Error = DRAW::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            let x0 = self.offset.x + self.scale_coord(point.x);
            let y0 = self.offset.y + self.scale_coord(point.y);
            let width = self.scale_span(point.x, point.x + 1);
            let height = self.scale_span(point.y, point.y + 1);

            if width == 0 || height == 0 {
                continue;
            }

            self.inner.fill_solid(
                &Rectangle::new(Point::new(x0, y0), Size::new(width, height)),
                color,
            )?;
        }

        Ok(())
    }
}

impl<DRAW> OriginDimensions for ScaledDrawTarget<'_, DRAW>
where
    DRAW: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    fn size(&self) -> Size {
        self.inner.size()
    }
}

fn render_scaled_text<DRAW>(
    display: &mut DRAW,
    sample_y: i32,
    scale_num: i32,
    scale_den: i32,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    let mut scaled = ScaledDrawTarget::new(display, Point::new(44, sample_y), scale_num, scale_den);
    BASE_FONT
        .render(
            DEMO_TEXT,
            Point::zero(),
            VerticalPosition::Top,
            FontColor::Transparent(SAMPLE_COLOR),
            &mut scaled,
        )
        .map(|_| ())
        .map_err(|err| match err {
            U8g2Error::DisplayError(display_err) => display_err,
            U8g2Error::BackgroundColorNotSupported => {
                panic!("Selected u8g2 font does not support background color")
            }
            U8g2Error::GlyphNotFound(glyph) => {
                panic!("Selected u8g2 font is missing glyph: {glyph:?}")
            }
        })?;

    Ok(())
}

fn render_sample<DRAW>(
    display: &mut DRAW,
    label: &str,
    sample_y: i32,
    scale_num: i32,
    scale_den: i32,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    let label_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(LABEL_COLOR)
        .build();

    Text::new(label, Point::new(18, sample_y + 12), label_style).draw(display)?;
    render_scaled_text(display, sample_y, scale_num, scale_den)
}

fn draw_demo<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    display.clear(BG_COLOR)?;

    Rectangle::new(
        Point::new(0, 0),
        Size::new(SCREEN_WIDTH as u32, HEADER_HEIGHT as u32),
    )
    .into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(HEADER_COLOR)
            .build(),
    )
    .draw(display)?;

    RoundedRectangle::with_equal_corners(
        Rectangle::new(Point::new(10, 72), Size::new(152, 162)),
        Size::new(12, 12),
    )
    .into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(CARD_FILL)
            .stroke_color(CARD_BORDER)
            .stroke_width(1)
            .build(),
    )
    .draw(display)?;

    let title_style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(TEXT_COLOR)
        .build();
    let body_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(TEXT_COLOR)
        .build();

    Text::new("u8g2-fonts", Point::new(10, 18), title_style).draw(display)?;
    Text::new("font demo", Point::new(10, 42), title_style).draw(display)?;
    Text::new("base: wqy16", Point::new(20, 88), body_style).draw(display)?;

    render_sample(display, "12", 102, 3, 4)?;
    render_sample(display, "16", 126, 1, 1)?;
    render_sample(display, "20", 156, 5, 4)?;
    render_sample(display, "24", 192, 3, 2)?;

    Ok(())
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    let _ = spawner;

    let mut delay = Delay::new();

    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .expect("SPI config failed")
    .with_sck(peripherals.GPIO1)
    .with_mosi(peripherals.GPIO2);

    let lcd_cs = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    let lcd_dc = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());
    let lcd_rst = Output::new(peripherals.GPIO22, Level::High, OutputConfig::default());
    let _lcd_bl = Output::new(peripherals.GPIO23, Level::High, OutputConfig::default());
    let spi_device = ExclusiveDevice::new_no_delay(spi, lcd_cs).expect("CS init failed");

    let display_config = Jd9853Config {
        invert_colors: true,
        reset_active_high: false,
        ..Jd9853Config::default()
    };
    let mut display = Jd9853::new(spi_device, lcd_dc, Some(lcd_rst), display_config);
    display.reset(&mut delay).expect("LCD reset failed");
    display.init(&mut delay).expect("LCD init failed");
    display.set_display_on(true).expect("LCD on failed");

    draw_demo(&mut display).expect("u8g2 demo draw failed");

    info!("u8g2 demo ready: text = {}", DEMO_TEXT);

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
