use block_compression::{decode::decompress_blocks, BlockCompressor, CompressionVariant, Settings};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use wgpu::{
    CommandEncoderDescriptor, ComputePassDescriptor, TextureFormat::Rgba8Unorm,
    TextureViewDescriptor,
};

use self::common::{
    create_blocks_buffer, create_wgpu_resources, download_blocks_data,
    read_image_and_create_texture, BRICK_FILE_PATH, MARBLE_FILE_PATH,
};

mod common;

#[derive(Debug, Clone)]
pub struct PsnrResult {
    pub overall_psnr: f64,
    pub overall_mse: f64,
    pub channel_results: ChannelResults,
}

#[derive(Debug, Clone)]
pub struct ChannelResults {
    pub red: ChannelMetrics,
    pub green: ChannelMetrics,
    pub blue: ChannelMetrics,
    pub alpha: ChannelMetrics,
}

#[derive(Debug, Clone)]
pub struct ChannelMetrics {
    pub psnr: f64,
    pub mse: f64,
}

/// Calculates quality metrics for a given image. The input data and output data must be RGBA data.
pub fn calculate_image_metrics(
    original: &[u8],
    compressed: &[u8],
    width: u32,
    height: u32,
    channels: u32,
) -> PsnrResult {
    if original.len() != compressed.len() {
        panic!("Image buffers must have same length");
    }
    if original.len() != (width * height * 4) as usize {
        panic!("Buffer size doesn't match dimensions");
    }

    let mut channel_mse = [0.0; 4];
    let pixel_count = (width * height) as f64;

    for index in (0..original.len()).step_by(4) {
        for channel in 0..4 {
            let orig = if channel < 3 {
                srgb_to_linear(original[index + channel])
            } else {
                (original[index + channel] as f64) / 255.0
            };

            let comp = if channel < 3 {
                srgb_to_linear(compressed[index + channel])
            } else {
                (compressed[index + channel] as f64) / 255.0
            };

            let diff = orig - comp;
            channel_mse[channel] += diff * diff;
        }
    }

    // Normalize MSE values
    channel_mse.iter_mut().for_each(|mse| *mse /= pixel_count);

    let calculate_psnr = |mse: f64| -> f64 {
        if mse == 0.0 {
            0.0
        } else {
            20.0 * (1.0 / mse.sqrt()).log10()
        }
    };

    let overall_mse = channel_mse.iter().sum::<f64>() / channels as f64;
    let overall_psnr = calculate_psnr(overall_mse);

    let channel_results = ChannelResults {
        red: ChannelMetrics {
            mse: channel_mse[0],
            psnr: calculate_psnr(channel_mse[0]),
        },
        green: ChannelMetrics {
            mse: channel_mse[1],
            psnr: calculate_psnr(channel_mse[1]),
        },
        blue: ChannelMetrics {
            mse: channel_mse[2],
            psnr: calculate_psnr(channel_mse[2]),
        },
        alpha: ChannelMetrics {
            mse: channel_mse[3],
            psnr: calculate_psnr(channel_mse[3]),
        },
    };

    PsnrResult {
        overall_psnr,
        overall_mse,
        channel_results,
    }
}

#[inline]
fn srgb_to_linear(srgb: u8) -> f64 {
    let v = (srgb as f64) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn print_metrics(name: &str, metrics: PsnrResult) {
    println!("-----------------------");
    println!("Image name: {}", name);
    println!("Overall PSNR: {:.2} dB", metrics.overall_psnr);
    println!("Overall MSE: {:.6}", metrics.overall_mse);
    println!(
        "Red channel PSNR: {:.2} dB",
        metrics.channel_results.red.psnr
    );
    println!(
        "Green channel PSNR: {:.2} dB",
        metrics.channel_results.green.psnr
    );
    println!(
        "Blue channel PSNR: {:.2} dB",
        metrics.channel_results.blue.psnr
    );
    println!(
        "Alpha channel PSNR: {:.2} dB",
        metrics.channel_results.alpha.psnr
    );
    println!("-----------------------");
}

fn save_png(filename: &str, data: &[u8], width: u32, height: u32) {
    let file = std::fs::File::create(filename).unwrap();
    let encoder = PngEncoder::new(file);
    encoder
        .write_image(data, width, height, ExtendedColorType::Rgba8)
        .unwrap();
}

fn calculate_psnr_for_image(
    image_path: &str,
    variant: CompressionVariant,
    channels: u32,
    settings: Option<Settings>,
) {
    let (device, queue) = create_wgpu_resources();
    let mut block_compressor = BlockCompressor::new(device.clone(), queue.clone());

    let (texture, original_data) = read_image_and_create_texture(&device, &queue, image_path);
    let blocks_size = variant.blocks_byte_size(texture.width(), texture.height());

    let blocks = create_blocks_buffer(&device, blocks_size as u64);

    block_compressor.add_compression_task(
        variant,
        &texture.create_view(&TextureViewDescriptor {
            format: Some(Rgba8Unorm),
            ..Default::default()
        }),
        texture.width(),
        texture.height(),
        &blocks,
        None,
        settings,
    );

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("command encoder"),
    });

    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("compute pass"),
            timestamp_writes: None,
        });

        block_compressor.compress(&mut pass);
    }

    queue.submit([encoder.finish()]);

    let blocks_data = download_blocks_data(&device, &queue, blocks);

    let size = texture.width() * texture.height() * 4;
    let mut decompressed_data = vec![0; size as usize];
    decompress_blocks(
        variant,
        texture.width(),
        texture.height(),
        &blocks_data,
        &mut decompressed_data,
    );

    let metrics = calculate_image_metrics(
        &original_data,
        &decompressed_data,
        texture.width(),
        texture.height(),
        channels,
    );

    let image_name = std::path::Path::new(image_path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();

    print_metrics(image_name, metrics);
}

#[test]
fn psnr_bc1() {
    calculate_psnr_for_image(BRICK_FILE_PATH, CompressionVariant::BC1, 3, None);
    calculate_psnr_for_image(MARBLE_FILE_PATH, CompressionVariant::BC1, 3, None);
}

#[test]
fn psnr_bc3() {
    calculate_psnr_for_image(BRICK_FILE_PATH, CompressionVariant::BC3, 4, None);
    calculate_psnr_for_image(MARBLE_FILE_PATH, CompressionVariant::BC3, 4, None);
}
