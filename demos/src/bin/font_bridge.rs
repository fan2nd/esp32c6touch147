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
    mono_font::{MonoTextStyleBuilder, ascii::FONT_10X20},
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
use lvgl_font_bridge::{EgTextStyle, FontPreset, lvgl_font};

use esp_backtrace as _;

const SCREEN_WIDTH: i32 = 172;
const HEADER_HEIGHT: i32 = 56;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const PANEL_COLOR: Rgb565 = Rgb565::new(4, 12, 18);
const PANEL_BORDER: Rgb565 = Rgb565::new(8, 20, 30);
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const SAMPLE_COLOR: Rgb565 = Rgb565::new(31, 58, 18);
const INVERSE_BG: Rgb565 = Rgb565::new(31, 58, 18);
const INVERSE_TEXT: Rgb565 = Rgb565::BLACK;
const DEMO_TEXT: &str = "rust, 牛逼";
const HELLO_FONT: FontPreset<'static> = lvgl_font!(
    path = "./src/asserts/hello.c",
    half_width = 18,
    full_width = 36,
    height = 36,
);

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
        Size::new((SCREEN_WIDTH - 16) as u32, 156),
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
    Text::new("lvgl-font-bridge", Point::new(10, 18), title_style).draw(display)?;
    Text::new("hello.c demo", Point::new(10, 42), title_style).draw(display)?;
    Text::new("glyphs:", Point::new(16, 96), title_style).draw(display)?;

    let hello_style = HELLO_FONT.default_text_style(SAMPLE_COLOR);
    let inverse_style = EgTextStyle::with_background(
        HELLO_FONT.font_data(),
        INVERSE_TEXT,
        INVERSE_BG,
        HELLO_FONT.height,
        HELLO_FONT.half_width,
        HELLO_FONT.full_width,
    );

    Text::with_baseline(DEMO_TEXT, Point::new(16, 132), hello_style, Baseline::Top)
        .draw(display)?;
    Text::with_baseline(DEMO_TEXT, Point::new(16, 172), inverse_style, Baseline::Top)
        .draw(display)?;
    Text::new("from hello.c", Point::new(16, 212), title_style).draw(display)?;
    Text::with_baseline(DEMO_TEXT, Point::new(16, 238), hello_style, Baseline::Top)
        .draw(display)?;

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
