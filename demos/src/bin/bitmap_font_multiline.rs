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
use embedded_bitmap_font::{DrawableText, FontData, VerticalDrawableText};
use embedded_bitmap_font_macros::font_data;
use embedded_graphics::{
    Drawable,
    mono_font::{MonoTextStyleBuilder, ascii::FONT_6X10},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
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

use esp_backtrace as _;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const SCREEN_WIDTH: i32 = 172;
const SCREEN_HEIGHT: i32 = 320;
const HEADER_HEIGHT: i32 = 44;
const PAGE_SECONDS: u64 = 4;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const PANEL_COLOR: Rgb565 = Rgb565::new(4, 12, 18);
const PANEL_BORDER: Rgb565 = Rgb565::new(8, 20, 30);
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const LABEL_COLOR: Rgb565 = Rgb565::new(18, 36, 44);
const HORIZONTAL_COLOR: Rgb565 = Rgb565::new(31, 52, 12);
const VERTICAL_COLOR: Rgb565 = Rgb565::new(10, 48, 31);
const BOX_COLOR: Rgb565 = Rgb565::new(31, 24, 4);
const HORIZONTAL_TEXT: &str = "Hello Rust\n你好 Rust";
const VERTICAL_TEXT: &str = "竖排\nRust\n你好";

static MULTILINE_FONT_18: FontData<'static> = font_data! {
    size: 18,
    path: "src/assets/unifont-17.0.04.otf",
    index: "Hello Rust\n你好竖排",
    y_offset: -2,
};

fn cell_sizes(font: &FontData<'_>) -> (Size, Size) {
    let cjk = Size::new(font.char_size as u32, font.char_size as u32);
    let ascii_width = (font.char_size as u32).saturating_sub(2).max(6);
    let ascii = Size::new(ascii_width, font.char_size as u32);
    (ascii, cjk)
}

fn draw_header<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
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

    let title_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(TEXT_COLOR)
        .build();
    let body_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(LABEL_COLOR)
        .build();

    Text::new("bitmap-font multiline", Point::new(10, 16), title_style).draw(display)?;
    Text::new("horizontal + vertical", Point::new(10, 32), body_style).draw(display)?;
    Ok(())
}

fn draw_horizontal_text_box<DRAW>(
    display: &mut DRAW,
    text: &'_ str,
    font: &'_ FontData<'_>,
    top_left: Point,
    text_color: Rgb565,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    let (ascii_cell_size, cjk_cell_size) = cell_sizes(font);
    let drawable = DrawableText::new(
        font,
        text,
        top_left,
        ascii_cell_size,
        cjk_cell_size,
        text_color,
    );
    let measured = drawable.measure();
    drawable.draw(display)?;
    Rectangle::new(top_left, measured)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .stroke_color(BOX_COLOR)
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;
    Ok(())
}

fn draw_vertical_text_box<DRAW>(
    display: &mut DRAW,
    text: &'_ str,
    font: &'_ FontData<'_>,
    top_left: Point,
    text_color: Rgb565,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    let (ascii_cell_size, cjk_cell_size) = cell_sizes(font);
    let drawable = VerticalDrawableText::new(
        font,
        text,
        top_left,
        ascii_cell_size,
        cjk_cell_size,
        text_color,
    );
    let measured = drawable.measure();
    drawable.draw(display)?;
    Rectangle::new(top_left, measured)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .stroke_color(BOX_COLOR)
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;
    Ok(())
}

fn draw_demo<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    display.clear(BG_COLOR)?;
    draw_header(display)?;

    Rectangle::new(
        Point::new(8, 58),
        Size::new((SCREEN_WIDTH - 16) as u32, (SCREEN_HEIGHT - 74) as u32),
    )
    .into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(PANEL_COLOR)
            .stroke_color(PANEL_BORDER)
            .stroke_width(1)
            .build(),
    )
    .draw(display)?;

    let label_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(LABEL_COLOR)
        .build();

    Text::new("DrawableText", Point::new(16, 84), label_style).draw(display)?;
    draw_horizontal_text_box(
        display,
        HORIZONTAL_TEXT,
        &MULTILINE_FONT_18,
        Point::new(16, 104),
        HORIZONTAL_COLOR,
    )?;

    Text::new("VerticalDrawableText", Point::new(16, 172), label_style).draw(display)?;
    draw_vertical_text_box(
        display,
        VERTICAL_TEXT,
        &MULTILINE_FONT_18,
        Point::new(16, 192),
        VERTICAL_COLOR,
    )?;

    Ok(())
}

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

    loop {
        draw_demo(&mut display).expect("bitmap font multiline demo draw failed");
        info!("embedded-bitmap-font multiline demo");
        Timer::after(Duration::from_secs(PAGE_SECONDS)).await;
    }
}
