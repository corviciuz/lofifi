use bytes::Bytes;
use crossterm::style::{Color, ResetColor, SetBackgroundColor, SetForegroundColor};
use image::{imageops::FilterType, DynamicImage, GenericImageView};

use crate::ArtStyle;
use super::cover::{rgb_to_color, rgb_to_gray};

const ASCII_GRADIENT: &[char] = &[
    '█', '▓', '▒', '░', '#', '&', '@', '%', '$', '*', '=', '+', ';', ':', '-', ',', '.', ' ',
];

pub struct AlbumCover {
    pub lines: Vec<String>,
}

impl AlbumCover {

    pub fn from_track_data(
        data: &Bytes,
        max_width: usize,
        style: &ArtStyle,
        colorize: bool,
    ) -> Option<Self> {
        let img = extract_cover_image(data)?;
        Some(Self::render(img, max_width, style, colorize))
    }

    pub fn from_image_data(
        image_data: &[u8],
        max_width: usize,
        style: &ArtStyle,
        colorize: bool,
    ) -> Option<Self> {
        match image::load_from_memory(image_data) {
            Ok(img) => Some(Self::render(img, max_width, style, colorize)),
            Err(_) => None
        }
    }

    fn render(img: DynamicImage, max_width: usize, style: &ArtStyle, colorize: bool) -> Self {
        let pixel_width = max_width / 2;
        let pixel_height = max_width / 2;

        let resized = img.resize_exact(
            pixel_width as u32,
            pixel_height as u32,
            FilterType::Lanczos3,
        );

        let lines = match style {
            ArtStyle::Pixel => render_pixel_art(&resized, max_width, colorize),
            ArtStyle::AsciiBg => render_ascii_art(&resized, max_width, colorize, false),
            ArtStyle::Ascii => render_ascii_art(&resized, max_width, colorize, true),
        };

        Self { lines }
    }
}

fn extract_cover_image(data: &Bytes) -> Option<DynamicImage> {
    if let Some(image_data) = super::cover::extract_image_from_tags(data) {
        image::load_from_memory(&image_data).ok()
    } else {
        None
    }
}

fn rgb_to_grayscale_color(rgb: [u8; 3]) -> Color {
    let gray = rgb_to_gray(rgb);
    Color::Rgb {
        r: gray,
        g: gray,
        b: gray,
    }
}

fn format_colored_block(color: Color) -> String {
    format!("{}  {}", SetBackgroundColor(color), ResetColor)
}

fn format_colored_ascii(color: Color, ch: char) -> String {
    format!("{}{}{}{}", SetForegroundColor(color), ch, ch, ResetColor)
}

fn format_bg_colored_ascii(color: Color, ch: char) -> String {
    format!("{}{}{}{}", SetBackgroundColor(color), ch, ch, ResetColor)
}

fn gray_to_ascii(gray: u8) -> char {
    let intensity = ((1.0 - (gray as f32 / 255.0)) * (ASCII_GRADIENT.len() - 1) as f32).round() as usize;
    ASCII_GRADIENT[intensity]
}

fn pad_line(line: String, current_width: usize, max_width: usize) -> String {
    if current_width < max_width {
        format!("{}{}", line, " ".repeat(max_width - current_width))
    } else {
        line
    }
}

pub fn render_pixel_art(img: &DynamicImage, max_width: usize, colorize: bool) -> Vec<String> {
    let mut lines = Vec::new();

    for y in 0..img.height() {
        let mut line = String::new();

        for x in 0..img.width() {
            let pixel = img.get_pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];

            let bg_color = if colorize {
                rgb_to_color(rgb)
            } else {
                rgb_to_grayscale_color(rgb)
            };
            line.push_str(&format_colored_block(bg_color));
        }

        let current_width = img.width() as usize * 2;
        lines.push(pad_line(line, current_width, max_width));
    }

    lines
}

pub fn render_ascii_art(
    img: &DynamicImage,
    max_width: usize,
    colorize: bool,
    use_foreground: bool,
) -> Vec<String> {
    let mut lines = Vec::new();

    for y in 0..img.height() {
        let mut line = String::new();

        for x in 0..img.width() {
            let pixel = img.get_pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];

            let gray = rgb_to_gray(rgb);
            let ch = gray_to_ascii(gray);

            if colorize {
                let color = rgb_to_color(rgb);
                if use_foreground {
                    line.push_str(&format_colored_ascii(color, ch));
                } else {
                    line.push_str(&format_bg_colored_ascii(color, ch));
                }
            } else {

                if use_foreground {

                    line.push_str(&format!("{}{}", ch, ch));
                } else {

                    let bg_color = rgb_to_grayscale_color(rgb);
                    line.push_str(&format_bg_colored_ascii(bg_color, ch));
                }
            }
        }

        let current_width = img.width() as usize * 2;
        lines.push(pad_line(line, current_width, max_width));
    }

    lines
}
