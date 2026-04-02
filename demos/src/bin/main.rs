#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use axs5106::{Axs5106, Axs5106Config};
use core::cell::RefCell;
use embedded_graphics::{
    mono_font::{
        MonoTextStyleBuilder,
        ascii::FONT_10X20,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_hal_bus::i2c::RefCellDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use jd9853::{Jd9853, Jd9853Config};
use qmi8658::{Qmi8658, Qmi8658Config, Vec3i16};

use log::info;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use esp_backtrace as _;

extern crate alloc;

const SCREEN_WIDTH: i32 = 172;
const SCREEN_HEIGHT: i32 = 320;
const HEADER_HEIGHT: i32 = 56;
const CIRCLE_DIAMETER: u32 = 36;
const LEVEL_DIAMETER: u32 = 24;
const CIRCLE_MARGIN: i32 = 3;
const MOVE_THRESHOLD: i32 = 3;
const BACKGROUND_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const CIRCLE1_FILL_COLOR: Rgb565 = Rgb565::new(28, 8, 8);
const LEVEL_FILL_COLOR: Rgb565 = Rgb565::new(6, 26, 8);
const CIRCLE_STROKE_COLOR: Rgb565 = Rgb565::WHITE;
const LEVEL_RANGE_RAW: i32 = 12_000;

fn clamp_ball_center(point: Point, diameter: u32) -> Point {
    let radius = (diameter as i32) / 2;
    let min_y = HEADER_HEIGHT + radius + CIRCLE_MARGIN;
    Point::new(
        point.x.clamp(radius + CIRCLE_MARGIN, SCREEN_WIDTH - radius - CIRCLE_MARGIN - 1),
        point.y.clamp(min_y, SCREEN_HEIGHT - radius - CIRCLE_MARGIN - 1),
    )
}

fn ball_bounds(center: Point, diameter: u32) -> Rectangle {
    let radius = (diameter as i32) / 2;
    Rectangle::new(
        Point::new(
            center.x - radius - CIRCLE_MARGIN,
            center.y - radius - CIRCLE_MARGIN,
        ),
        Size::new(
            diameter + (CIRCLE_MARGIN as u32) * 2,
            diameter + (CIRCLE_MARGIN as u32) * 2,
        ),
    )
}

fn content_bounds() -> Rectangle {
    Rectangle::new(
        Point::new(0, HEADER_HEIGHT),
        Size::new(SCREEN_WIDTH as u32, (SCREEN_HEIGHT - HEADER_HEIGHT) as u32),
    )
}

fn draw_static_ui<DRAW>(display: &mut DRAW) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    display.clear(BACKGROUND_COLOR)?;

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
        .text_color(Rgb565::WHITE)
        .build();

    Text::new("Touch Demo", Point::new(10, 18), text_style).draw(display)?;
    Text::new("Move the circle", Point::new(10, 42), text_style).draw(display)?;

    Ok(())
}

fn draw_circle<DRAW>(
    display: &mut DRAW,
    circle_center: Point,
    diameter: u32,
    fill: Rgb565,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    Circle::with_center(circle_center, diameter)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .stroke_color(CIRCLE_STROKE_COLOR)
                .stroke_width(3)
                .fill_color(fill)
                .build(),
        )
        .draw(display)?;

    Ok(())
}

fn erase_circle<DRAW>(display: &mut DRAW, circle_center: Point, diameter: u32) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    let erase_area = ball_bounds(circle_center, diameter).intersection(&content_bounds());
    if erase_area.size.width == 0 || erase_area.size.height == 0 {
        return Ok(());
    }

    erase_area
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(BACKGROUND_COLOR)
                .build(),
        )
        .draw(display)
}

fn moved_enough(old: Point, new: Point) -> bool {
    (new.x - old.x).abs() >= MOVE_THRESHOLD || (new.y - old.y).abs() >= MOVE_THRESHOLD
}

fn level_target_from_accel(accel: Vec3i16) -> Point {
    let center_x = SCREEN_WIDTH / 2;
    let center_y = HEADER_HEIGHT + (SCREEN_HEIGHT - HEADER_HEIGHT) / 2;
    let radius = (LEVEL_DIAMETER as i32) / 2;
    let travel_x = (SCREEN_WIDTH / 2) - radius - CIRCLE_MARGIN - 2;
    let travel_y = ((SCREEN_HEIGHT - HEADER_HEIGHT) / 2) - radius - CIRCLE_MARGIN - 2;

    let ax = (accel.x as i32).clamp(-LEVEL_RANGE_RAW, LEVEL_RANGE_RAW);
    let ay = (accel.y as i32).clamp(-LEVEL_RANGE_RAW, LEVEL_RANGE_RAW);
    let dx = -(ax * travel_x / LEVEL_RANGE_RAW);
    let dy = -ay * travel_y / LEVEL_RANGE_RAW;

    clamp_ball_center(Point::new(center_x + dx, center_y + dy), LEVEL_DIAMETER)
}

fn smooth_step(current: Point, target: Point) -> Point {
    Point::new(
        current.x + (target.x - current.x) / 4,
        current.y + (target.y - current.y) / 4,
    )
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
    // generator version: 1.2.0

    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    let _ = spawner;

    info!("Initializing JD9853 + AXS5106 + QMI8658 demo");

    let mut delay = Delay::new();

    // Adjust these pins to match your board wiring.
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

    let mut touch_visible = false;
    let mut touch_circle = clamp_ball_center(
        Point::new(SCREEN_WIDTH / 2, (SCREEN_HEIGHT / 2) + 20),
        CIRCLE_DIAMETER,
    );
    let mut level_circle = clamp_ball_center(
        Point::new(SCREEN_WIDTH / 2, (SCREEN_HEIGHT / 2) + 20),
        LEVEL_DIAMETER,
    );
    draw_static_ui(&mut display).expect("Static UI render failed");
    draw_circle(&mut display, level_circle, LEVEL_DIAMETER, LEVEL_FILL_COLOR)
        .expect("Level circle draw failed");

    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .expect("I2C config failed")
    .with_sda(peripherals.GPIO18)
    .with_scl(peripherals.GPIO19);
    let i2c_bus = RefCell::new(i2c);
    let touch_i2c = RefCellDevice::new(&i2c_bus);
    let imu_i2c = RefCellDevice::new(&i2c_bus);

    let touch_rst = Output::new(peripherals.GPIO20, Level::High, OutputConfig::default());
    let _touch_int = Input::new(
        peripherals.GPIO21,
        InputConfig::default().with_pull(Pull::Up),
    );

    let touch_config = Axs5106Config {
        mirror_x: true,
        reset_active_high: false,
        ..Axs5106Config::default()
    };
    let mut touch = Axs5106::new(touch_i2c, Some(touch_rst), touch_config);
    touch.reset(&mut delay).expect("Touch reset failed");
    touch.init().expect("Touch init failed");

    let mut imu = Qmi8658::new(imu_i2c, Qmi8658Config::default());
    let mut imu_ready = false;
    match imu.init(&mut delay) {
        Ok(()) => {
            imu_ready = true;
            info!("QMI8658 initialized at 0x6B");
        }
        Err(error_6b) => {
            let mut cfg_6a = Qmi8658Config::default();
            cfg_6a.address = 0x6A;
            let mut imu_alt = Qmi8658::new(imu.release(), cfg_6a);
            match imu_alt.init(&mut delay) {
                Ok(()) => {
                    info!("QMI8658 initialized at 0x6A");
                    imu = imu_alt;
                    imu_ready = true;
                }
                Err(error_6a) => {
                    log::warn!(
                        "QMI8658 init failed (0x6B: {:?}, 0x6A: {:?}), level ball disabled",
                        error_6b,
                        error_6a
                    );
                    imu = imu_alt;
                }
            }
        }
    }

    loop {
        let mut next_touch = touch_circle;
        let mut next_touch_visible = touch_visible;

        match touch.read_touches() {
            Ok(Some(point)) => {
                next_touch = clamp_ball_center(
                    Point::new(point.x as i32, point.y as i32),
                    CIRCLE_DIAMETER,
                );
                next_touch_visible = true;
            }
            Ok(None) => {
                next_touch_visible = false;
            }
            Err(error) => {
                log::warn!("touch read failed: {:?}", error);
            }
        }

        let mut next_level = level_circle;
        if imu_ready {
            match imu.read_data_if_ready() {
                Ok(Some(data)) => {
                    let target = level_target_from_accel(data.accel);
                    next_level = smooth_step(level_circle, target);
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!("imu read failed: {:?}", error);
                }
            }
        }

        let level_changed = moved_enough(level_circle, next_level);
        let touch_changed = (touch_visible != next_touch_visible)
            || (next_touch_visible && moved_enough(touch_circle, next_touch));

        if level_changed || touch_changed {
            erase_circle(&mut display, level_circle, LEVEL_DIAMETER).expect("Level erase failed");
            if touch_visible {
                erase_circle(&mut display, touch_circle, CIRCLE_DIAMETER).expect("Touch erase failed");
            }

            level_circle = next_level;
            touch_circle = next_touch;
            touch_visible = next_touch_visible;

            draw_circle(&mut display, level_circle, LEVEL_DIAMETER, LEVEL_FILL_COLOR)
                .expect("Level draw failed");
            if touch_visible {
                draw_circle(&mut display, touch_circle, CIRCLE_DIAMETER, CIRCLE1_FILL_COLOR)
                    .expect("Touch draw failed");
            }
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}
