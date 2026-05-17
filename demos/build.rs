use embedded_bitmap_font_codegen::{BitmapGlyph, CodegenFont, FontWriter, GlyphBitmap};
use fontdue::{Font, FontSettings};
use std::{env, fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    linker_be_nice();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    write_demo_font(
        "cubic_font.rs",
        font_from_file("CUBIC_DEMO_FONT", "src/assets/Cubic_11.ttf", 18.0, 18)?,
    )?;
    write_demo_font(
        "unifont_demo_font.rs",
        font_from_file(
            "UNIFONT_DEMO_FONT",
            "src/assets/unifont-17.0.04.otf",
            18.0,
            18,
        )?,
    )?;
    println!("cargo:rerun-if-changed=src/assets/Cubic_11.ttf");
    println!("cargo:rerun-if-changed=src/assets/unifont-17.0.04.otf");

    Ok(())
}

fn write_demo_font(file_name: &str, font: CodegenFont) -> io::Result<()> {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let source = FontWriter::new(font)
        .write_rust_source()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "font source format error"))?;
    fs::write(out_dir.join(file_name), source)
}

fn font_from_file(ident: &str, path: &str, px: f32, size: u16) -> io::Result<CodegenFont> {
    let bytes = fs::read(path)?;
    let font = Font::from_bytes(bytes, FontSettings::default())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let glyphs = ['H', 'e', 'l', 'o', ' ', 'R', 'u', 's', 't', '你', '好']
        .into_iter()
        .map(|codepoint| rasterize_glyph(&font, codepoint, px))
        .collect();

    Ok(CodegenFont {
        ident: ident.to_string(),
        size,
        ascent: (size as i16) - 3,
        descent: -3,
        line_gap: 2,
        glyphs,
    })
}

fn rasterize_glyph(font: &Font, codepoint: char, px: f32) -> BitmapGlyph {
    let (metrics, bitmap) = font.rasterize(codepoint, px);
    let width = metrics.width.max(1) as u16;
    let height = metrics.height.max(1) as u16;
    let pixels = if metrics.width == 0 || metrics.height == 0 {
        vec![false; width as usize * height as usize]
    } else {
        bitmap.into_iter().map(|alpha| alpha >= 96).collect()
    };

    BitmapGlyph {
        codepoint,
        width,
        height,
        x_offset: metrics.xmin as i16,
        y_offset: (metrics.height as i32 + metrics.ymin) as i16,
        x_advance: metrics.advance_width.ceil().max(width as f32) as i16,
        bitmap: GlyphBitmap::Bpp1(pixels),
    }
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
