use std::{fs, path::Path};

use anyhow::{Context, Result};
use image::{Rgb, RgbImage};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;

fn main() -> Result<()> {
    let output = Path::new("tests/fixtures/vlm");
    fs::create_dir_all(output).context("create fixture directory")?;

    colors_and_shapes()
        .save(output.join("colors-and-shapes.png"))
        .context("save colors-and-shapes fixture")?;
    simple_landscape()
        .save(output.join("simple-landscape.png"))
        .context("save simple-landscape fixture")?;
    counting_grid()
        .save(output.join("counting-grid.png"))
        .context("save counting-grid fixture")?;

    println!("generated fixtures in {}", output.display());
    Ok(())
}

fn colors_and_shapes() -> RgbImage {
    let mut image = RgbImage::from_pixel(WIDTH, HEIGHT, Rgb([245, 245, 240]));
    circle(&mut image, 180, 200, 100, Rgb([220, 38, 38]));
    rectangle(&mut image, 380, 100, 200, 200, Rgb([37, 99, 235]));
    image
}

fn simple_landscape() -> RgbImage {
    let mut image = RgbImage::from_pixel(WIDTH, HEIGHT, Rgb([125, 211, 252]));
    rectangle(&mut image, 0, 280, WIDTH, 120, Rgb([34, 197, 94]));
    circle(&mut image, 530, 85, 52, Rgb([250, 204, 21]));
    rectangle(&mut image, 235, 190, 180, 150, Rgb([180, 83, 9]));
    triangle(
        &mut image,
        (210, 195),
        (325, 105),
        (440, 195),
        Rgb([185, 28, 28]),
    );
    rectangle(&mut image, 305, 255, 45, 85, Rgb([69, 26, 3]));
    rectangle(&mut image, 255, 220, 40, 40, Rgb([224, 242, 254]));
    rectangle(&mut image, 365, 220, 40, 40, Rgb([224, 242, 254]));
    image
}

fn counting_grid() -> RgbImage {
    let mut image = RgbImage::from_pixel(WIDTH, HEIGHT, Rgb([255, 255, 255]));
    for (x, color) in [
        (140, Rgb([239, 68, 68])),
        (320, Rgb([34, 197, 94])),
        (500, Rgb([59, 130, 246])),
    ] {
        circle(&mut image, x, 135, 65, color);
    }
    rectangle(&mut image, 200, 260, 90, 90, Rgb([17, 24, 39]));
    rectangle(&mut image, 350, 260, 90, 90, Rgb([17, 24, 39]));
    image
}

fn rectangle(image: &mut RgbImage, x: u32, y: u32, width: u32, height: u32, color: Rgb<u8>) {
    for pixel_y in y..(y + height).min(image.height()) {
        for pixel_x in x..(x + width).min(image.width()) {
            image.put_pixel(pixel_x, pixel_y, color);
        }
    }
}

fn circle(image: &mut RgbImage, center_x: u32, center_y: u32, radius: u32, color: Rgb<u8>) {
    let radius_squared = i64::from(radius).pow(2);
    for y in center_y.saturating_sub(radius)..(center_y + radius).min(image.height()) {
        for x in center_x.saturating_sub(radius)..(center_x + radius).min(image.width()) {
            let dx = i64::from(x) - i64::from(center_x);
            let dy = i64::from(y) - i64::from(center_y);
            if dx * dx + dy * dy <= radius_squared {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn triangle(image: &mut RgbImage, a: (u32, u32), b: (u32, u32), c: (u32, u32), color: Rgb<u8>) {
    let min_x = a.0.min(b.0).min(c.0);
    let max_x = a.0.max(b.0).max(c.0).min(image.width() - 1);
    let min_y = a.1.min(b.1).min(c.1);
    let max_y = a.1.max(b.1).max(c.1).min(image.height() - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if inside_triangle((x, y), a, b, c) {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn inside_triangle(point: (u32, u32), a: (u32, u32), b: (u32, u32), c: (u32, u32)) -> bool {
    fn sign(p1: (u32, u32), p2: (u32, u32), p3: (u32, u32)) -> i64 {
        (i64::from(p1.0) - i64::from(p3.0)) * (i64::from(p2.1) - i64::from(p3.1))
            - (i64::from(p2.0) - i64::from(p3.0)) * (i64::from(p1.1) - i64::from(p3.1))
    }

    let d1 = sign(point, a, b);
    let d2 = sign(point, b, c);
    let d3 = sign(point, c, a);
    !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0))
}
