use block_compression::{decode::decompress_blocks, BlockCompressor, CompressionVariant, Settings};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use wgpu::{
    CommandEncoderDescriptor, ComputePassDescriptor, TextureFormat::Rgba8Unorm,
    TextureViewDescriptor,
};

use crate::common::{
    create_blocks_buffer, create_wgpu_resources, download_blocks_data,
    metrics::{calculate_image_metrics, PsnrResult},
    read_image_and_create_texture, BRICK_FILE_PATH, MARBLE_FILE_PATH,
};

mod common;

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
