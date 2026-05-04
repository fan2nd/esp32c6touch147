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
    mono_font::{
        MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
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
use lvgl_font_bridge::{FontData, FontPreset, lvgl_font};

use esp_backtrace as _;

const SCREEN_WIDTH: i32 = 172;
const HEADER_HEIGHT: i32 = 56;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const PANEL_COLOR: Rgb565 = Rgb565::new(4, 12, 18);
const PANEL_BORDER: Rgb565 = Rgb565::new(8, 20, 30);
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const LABEL_COLOR: Rgb565 = Rgb565::new(18, 36, 44);
const SAMPLE_COLOR: Rgb565 = Rgb565::new(31, 58, 18);
const DEMO_TEXT: &str = "rust,牛逼!";

const ORIGIN_FONT: FontData<'static> = lvgl_font!(
    path = "./src/asserts/hello.c",
    half_width = 8,
    full_width = 16,
    height = 16,
);

const HELLO_FONT_12: FontPreset<'static> = FontPreset::new(&ORIGIN_FONT).with_scaled_height(12);
const HELLO_FONT_16: FontPreset<'static> = FontPreset::new(&ORIGIN_FONT).with_scaled_height(16);
const HELLO_FONT_20: FontPreset<'static> = FontPreset::new(&ORIGIN_FONT).with_scaled_height(20);
const HELLO_FONT_24: FontPreset<'static> = FontPreset::new(&ORIGIN_FONT).with_scaled_height(24);

fn scaled_cell_width(character: char, scaled_height: u32) -> i32 {
    let base_width = if character.is_ascii() { 8 } else { 16 };
    (((base_width * scaled_height) + 8) / 16) as i32
}

fn draw_scaled_sample<DRAW>(
    display: &mut DRAW,
    label: &str,
    sample_y: i32,
    scaled_height: u32,
    font: &FontPreset<'static>,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    let label_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(LABEL_COLOR)
        .build();
    let text_style = font.text_style(SAMPLE_COLOR, scaled_height);
    let mut cursor_x = 44;

    Text::new(label, Point::new(18, sample_y + 10), label_style).draw(display)?;

    for character in DEMO_TEXT.chars() {
        let mut encoded = [0_u8; 4];
        let glyph = character.encode_utf8(&mut encoded);

        Text::with_baseline(
            glyph,
            Point::new(cursor_x, sample_y),
            text_style,
            Baseline::Top,
        )
        .draw(display)?;
        cursor_x += scaled_cell_width(character, scaled_height);
    }

    Ok(())
}

fn draw_demo<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
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

    Rectangle::new(
        Point::new(8, 72),
        Size::new((SCREEN_WIDTH - 16) as u32, 170),
    )
    .into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(PANEL_COLOR)
            .stroke_color(PANEL_BORDER)
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

    Text::new("lvgl-font-bridge", Point::new(10, 18), title_style).draw(display)?;
    Text::new("scaled_height demo", Point::new(10, 42), title_style).draw(display)?;
    Text::new("base bitmap: 8 / 16 / 16", Point::new(16, 90), body_style).draw(display)?;

    draw_scaled_sample(display, "12", 108, 12, &HELLO_FONT_12)?;
    draw_scaled_sample(display, "16", 134, 16, &HELLO_FONT_16)?;
    draw_scaled_sample(display, "20", 164, 20, &HELLO_FONT_20)?;
    draw_scaled_sample(display, "24", 198, 24, &HELLO_FONT_24)?;

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

    draw_demo(&mut display).expect("Font demo draw failed");

    info!("lvgl-font-bridge demo ready: text = {}", DEMO_TEXT);

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
