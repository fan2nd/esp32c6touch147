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
use qr::{Ecc, EncodeOptions, QrEncoder, Version3, VersionTrait};

use esp_backtrace as _;

const SCREEN_WIDTH: i32 = 172;
const SCREEN_HEIGHT: i32 = 320;
const HEADER_HEIGHT: i32 = 56;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const QR_LIGHT: Rgb565 = Rgb565::WHITE;
const QR_DARK: Rgb565 = Rgb565::BLACK;
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;

fn draw_background<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
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

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(TEXT_COLOR)
        .build();

    Text::new("hello embassy", Point::new(10, 18), text_style).draw(display)?;
    Text::new("QR encode_draw", Point::new(10, 42), text_style).draw(display)?;

    Ok(())
}

fn draw_qr_encode_draw<DRAW>(
    display: &mut DRAW,
    encoder: &QrEncoder<Version3>,
    payload: &str,
) -> Result<(), qr::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    let modules = <Version3 as VersionTrait>::SIZE as i32;
    let quiet_modules = 4;
    let full_modules = modules + quiet_modules * 2;

    let available_w = SCREEN_WIDTH - 16;
    let available_h = SCREEN_HEIGHT - HEADER_HEIGHT - 16;
    let scale = (available_w / full_modules)
        .min(available_h / full_modules)
        .max(1);

    let qr_side = full_modules * scale;
    let origin_x = (SCREEN_WIDTH - qr_side) / 2;
    let origin_y = HEADER_HEIGHT + (SCREEN_HEIGHT - HEADER_HEIGHT - qr_side) / 2;

    Rectangle::new(
        Point::new(origin_x, origin_y),
        Size::new(qr_side as u32, qr_side as u32),
    )
    .into_styled(PrimitiveStyleBuilder::new().fill_color(QR_LIGHT).build())
    .draw(display)
    .ok();

    let dark_style = PrimitiveStyleBuilder::new().fill_color(QR_DARK).build();
    let mut draw_failed = false;

    encoder.draw_str(payload, EncodeOptions { ecc: Ecc::M }, |x, y, dark| {
        if draw_failed || !dark {
            return;
        }

        let px = origin_x + (x as i32 + quiet_modules) * scale;
        let py = origin_y + (y as i32 + quiet_modules) * scale;

        if Rectangle::new(Point::new(px, py), Size::new(scale as u32, scale as u32))
            .into_styled(dark_style)
            .draw(display)
            .is_err()
        {
            draw_failed = true;
        }
    })?;

    if draw_failed {
        panic!("QR encode_draw render failed");
    }

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

    let encoder = QrEncoder::<Version3>::new();

    draw_background(&mut display).expect("Background draw failed");
    draw_qr_encode_draw(&mut display, &encoder, "hello embassy").expect("QR encode_draw failed");

    info!("QR encode_draw demo ready: payload = \"hello embassy\", version = 3, ecc = M");

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
