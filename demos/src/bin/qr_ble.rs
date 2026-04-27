#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::Delay;
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_qr::{EccLevel, QrBuilder, QrMatrix, Version, Version7};
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_radio::ble::controller::BleConnector;
use jd9853::{Jd9853, Jd9853Config};
use log::{info, warn};
use trouble_host::prelude::*;

use esp_backtrace as _;

extern crate alloc;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

const SCREEN_WIDTH: i32 = 172;
const SCREEN_HEIGHT: i32 = 320;
const HEADER_HEIGHT: i32 = 56;
const BG_COLOR: Rgb565 = Rgb565::new(2, 6, 10);
const HEADER_COLOR: Rgb565 = Rgb565::BLACK;
const QR_LIGHT: Rgb565 = Rgb565::WHITE;
const QR_DARK: Rgb565 = Rgb565::BLACK;
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const BLE_DOT_CONNECTED: Rgb565 = Rgb565::new(0, 52, 0);
const BLE_DOT_DISCONNECTED: Rgb565 = Rgb565::new(52, 0, 0);
const BLE_DOT_DIAMETER: u32 = 10;
const BLE_DOT_RIGHT_INSET: i32 = 16;
const BLE_DOT_TOP_INSET: i32 = 12;

const DEFAULT_PAYLOAD: &str = "hello embassy";
const DEVICE_NAME_BYTES: &[u8] = b"ESP32C6-QR";
const DEVICE_NAME: &str = "ESP32C6-QR";
const QR_TEXT_MAX: usize = 96;

#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct QrTextService {
    #[characteristic(
        uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e",
        read,
        write,
        value = [0; QR_TEXT_MAX]
    )]
    text: [u8; QR_TEXT_MAX],
}

#[gatt_server]
struct QrBleServer {
    qr: QrTextService,
}

fn trim_payload(data: &[u8]) -> &[u8] {
    let mut end = data.len();
    while end > 0 {
        let b = data[end - 1];
        if b == 0 || b == b'\r' || b == b'\n' {
            end -= 1;
        } else {
            break;
        }
    }
    &data[..end]
}

fn draw_background<DRAW>(display: &mut DRAW, status_line: &str) -> Result<(), DRAW::Error>
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

    Text::new("BLE QR Demo", Point::new(10, 18), text_style).draw(display)?;
    Text::new(status_line, Point::new(10, 42), text_style).draw(display)?;

    Ok(())
}

fn draw_ble_indicator<DRAW>(display: &mut DRAW, connected: bool) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    // Keep enough inset to avoid the rounded top-right screen corner.
    let center = Point::new(SCREEN_WIDTH - BLE_DOT_RIGHT_INSET, BLE_DOT_TOP_INSET);
    let color = if connected {
        BLE_DOT_CONNECTED
    } else {
        BLE_DOT_DISCONNECTED
    };

    Circle::with_center(center, BLE_DOT_DIAMETER)
        .into_styled(PrimitiveStyleBuilder::new().fill_color(color).build())
        .draw(display)?;

    Ok(())
}

fn draw_qr<DRAW, V>(display: &mut DRAW, qr: &QrMatrix<V>) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
    V: Version,
{
    let modules = qr.width() as i32;
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
    .draw(display)?;

    let dark_style = PrimitiveStyleBuilder::new().fill_color(QR_DARK).build();
    for y in 0..modules {
        for x in 0..modules {
            if qr.get(x as usize, y as usize) {
                let px = origin_x + (x + quiet_modules) * scale;
                let py = origin_y + (y + quiet_modules) * scale;

                Rectangle::new(Point::new(px, py), Size::new(scale as u32, scale as u32))
                    .into_styled(dark_style)
                    .draw(display)?;
            }
        }
    }

    Ok(())
}

fn redraw_screen<DRAW>(
    display: &mut DRAW,
    payload: &str,
    ble_connected: bool,
) -> Result<(), DRAW::Error>
where
    DRAW: DrawTarget<Color = Rgb565>,
{
    draw_background(display, "write text over BLE")?;
    draw_ble_indicator(display, ble_connected)?;

    let qr_matrix = QrBuilder::<Version7>::new()
        .with_ecc_level(EccLevel::M)
        .build(payload.as_bytes());
    if let Ok(qr_matrix) = qr_matrix {
        draw_qr(display, &qr_matrix)?;
    } else {
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(TEXT_COLOR)
            .build();
        Text::new("QR encode failed", Point::new(10, 96), text_style).draw(display)?;
        Text::new("text too long", Point::new(10, 120), text_style).draw(display)?;
    }

    Ok(())
}

async fn run_ble_stack<C, P>(mut runner: Runner<'_, C, P>)
where
    C: Controller,
    P: PacketPool,
{
    loop {
        if let Err(error) = runner.run().await {
            warn!("BLE runner stopped: {:?}", error);
        }
    }
}

async fn advertise_and_connect<'stack, 'server, C>(
    peripheral: &mut Peripheral<'stack, C, DefaultPacketPool>,
    server: &'server QrBleServer<'stack>,
) -> Result<GattConnection<'stack, 'server, DefaultPacketPool>, BleHostError<C::Error>>
where
    C: Controller,
{
    let mut adv_data = [0; 31];
    let mut scan_data = [0; 31];

    let adv_data_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[[
                0x9e, 0xca, 0xdc, 0x24, 0x0e, 0xe5, 0xa9, 0xe0, 0x93, 0xf3, 0xa3, 0xb5, 0x01, 0x00,
                0x40, 0x6e,
            ]]),
        ],
        &mut adv_data[..],
    )?;
    let scan_data_len = AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(DEVICE_NAME_BYTES)],
        &mut scan_data[..],
    )?;

    let advertisement = Advertisement::ConnectableScannableUndirected {
        adv_data: &adv_data[..adv_data_len],
        scan_data: &scan_data[..scan_data_len],
    };

    let advertiser = peripheral
        .advertise(&Default::default(), advertisement)
        .await?;
    let conn = advertiser.accept().await?;
    conn.with_attribute_server(server)
        .map_err(BleHostError::from)
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

    let mut delay = Delay;

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

    redraw_screen(&mut display, DEFAULT_PAYLOAD, false).expect("Initial QR draw failed");

    let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    let transport = BleConnector::new(&radio_init, peripherals.BT, Default::default())
        .expect("BLE transport init failed");
    let ble_controller = ExternalController::<_, 1>::new(transport);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(ble_controller, &mut resources);
    let Host {
        runner,
        mut peripheral,
        ..
    } = stack.build();

    let server = QrBleServer::new_default(DEVICE_NAME).expect("Failed to build GATT server");

    let mut current_payload = [0u8; QR_TEXT_MAX];
    current_payload[..DEFAULT_PAYLOAD.len()].copy_from_slice(DEFAULT_PAYLOAD.as_bytes());
    let mut current_len = DEFAULT_PAYLOAD.len();

    info!("BLE QR demo ready, name = {}", DEVICE_NAME);

    let app = async {
        loop {
            info!("Advertising BLE peripheral");
            let conn = match advertise_and_connect(&mut peripheral, &server).await {
                Ok(conn) => conn,
                Err(error) => {
                    warn!("Advertising failed: {:?}", error);
                    continue;
                }
            };
            info!("BLE connected");
            let _ = redraw_screen(
                &mut display,
                core::str::from_utf8(&current_payload[..current_len]).unwrap_or(DEFAULT_PAYLOAD),
                true,
            );

            loop {
                match conn.next().await {
                    GattConnectionEvent::Disconnected { reason } => {
                        info!("BLE disconnected: {:?}", reason);
                        let _ = redraw_screen(
                            &mut display,
                            core::str::from_utf8(&current_payload[..current_len])
                                .unwrap_or(DEFAULT_PAYLOAD),
                            false,
                        );
                        break;
                    }
                    GattConnectionEvent::Gatt { event } => {
                        if let GattEvent::Write(write) = &event {
                            if write.handle() == server.qr.text.handle {
                                let trimmed = trim_payload(write.data());
                                if trimmed.is_empty() {
                                    warn!("Received empty BLE text, ignored");
                                } else if trimmed.len() > QR_TEXT_MAX {
                                    warn!(
                                        "BLE text too long ({} > {}), ignored",
                                        trimmed.len(),
                                        QR_TEXT_MAX
                                    );
                                } else if core::str::from_utf8(trimmed).is_err() {
                                    warn!("BLE text is not valid UTF-8, ignored");
                                } else {
                                    let changed = current_len != trimmed.len()
                                        || current_payload[..current_len] != trimmed[..];
                                    if changed {
                                        current_payload[..trimmed.len()].copy_from_slice(trimmed);
                                        current_len = trimmed.len();

                                        let new_text =
                                            core::str::from_utf8(&current_payload[..current_len])
                                                .unwrap_or(DEFAULT_PAYLOAD);

                                        info!("QR payload updated over BLE: {}", new_text);
                                        if redraw_screen(&mut display, new_text, true).is_err() {
                                            warn!("Display refresh failed");
                                        }
                                    }
                                }
                            }
                        }

                        if let Ok(reply) = event.accept() {
                            reply.send().await;
                        }
                    }
                    _ => {}
                }
            }
        }
    };

    join(run_ble_stack(runner), app).await;

    loop {}
}
