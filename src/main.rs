use bytemuck::cast_slice;
use ddsfile::{AlphaMode, D3D10ResourceDimension, Dds, NewDxgiParams};
use image::{EncodableLayout, ImageReader};
use pollster::block_on;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use wgpu::util::{DeviceExt, TextureDataOrder};
use wgpu::{
    Backends, Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
    ComputePassDescriptor, ComputePassTimestampWrites, Device, DeviceDescriptor, Dx12Compiler,
    Extent3d, Features, Gles3MinorVersion, Instance, InstanceDescriptor, InstanceFlags, Limits,
    Maintain, MapMode, MemoryHints, QueryType, Queue, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureViewDescriptor,
};

mod block_compressor;
mod settings;

pub use block_compressor::{BlockCompressor, CompressionVariant};
pub use settings::{BC6HSettings, BC7Settings};

// TODO: NHA Implement BC6H
// TODO: NHA Implement BC7
// TODO: Properly crate layout as a lib / with extra bin project
// TODO: Documentation
// TODO: Decide on the error model

fn main() {
    let file_name = "input4096alpha.png".to_string();
    let variant = CompressionVariant::BC3;

    let (device, queue) = create_resources();
    let mut compressor: BlockCompressor = BlockCompressor::new(device.clone(), queue.clone());

    let start = Instant::now();

    let texture = read_image_and_create_texture(&device, &queue, &file_name);
    let texture_view = texture.create_view(&TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    let width = texture.width();
    let height = texture.height();

    let duration = start.elapsed();
    println!(
        "Image read and upload took: {:.3} ms",
        duration.as_secs_f64() * 1000.0
    );

    let blocks_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("blocks buffer"),
        size: variant.blocks_byte_size(width, height) as _,
        usage: BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    match variant {
        CompressionVariant::BC6H => {
            compressor.add_compression_task(
                variant,
                &texture_view,
                width,
                height,
                &blocks_buffer,
                None,
                BC6HSettings::slow(),
            );
        }
        CompressionVariant::BC7 => {
            compressor.add_compression_task(
                variant,
                &texture_view,
                width,
                height,
                &blocks_buffer,
                None,
                BC7Settings::alpha_slow(),
            );
        }
        _ => {
            compressor.add_compression_task(
                variant,
                &texture_view,
                width,
                height,
                &blocks_buffer,
                None,
                None,
            );
        }
    }

    compressor.upload();
    compress(&mut compressor, &device, &queue);

    let start = Instant::now();

    let block_data = download_blocks_data(&device, &queue, blocks_buffer);

    let duration = start.elapsed();
    println!(
        "Block data download took: {:.3} ms",
        duration.as_secs_f64() * 1000.0
    );

    let start = Instant::now();

    write_dds_file(&file_name, variant, width, height, block_data);

    let duration = start.elapsed();
    println!(
        "DDS output to disk took: {:.3} ms",
        duration.as_secs_f64() * 1000.0
    );
}

fn create_resources() -> (Arc<Device>, Arc<Queue>) {
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::all(),
        flags: InstanceFlags::default(),
        dx12_shader_compiler: Dx12Compiler::default(),
        gles_minor_version: Gles3MinorVersion::default(),
    });

    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("Failed to find an appropriate adapter");

    let (device, queue) = block_on(adapter.request_device(
        &DeviceDescriptor {
            label: Some("main device"),
            required_features: Features::TIMESTAMP_QUERY | Features::TEXTURE_COMPRESSION_BC,
            required_limits: Limits::default(),
            memory_hints: MemoryHints::default(),
        },
        None,
    ))
    .expect("Failed to create device");

    (Arc::new(device), Arc::new(queue))
}

fn read_image_and_create_texture(device: &Device, queue: &Queue, file_name: &str) -> Texture {
    let image = ImageReader::open(file_name)
        .expect("can't open input image")
        .decode()
        .expect("can't decode image");

    let rgba_image = image.to_rgba8();
    let width = rgba_image.width();
    let height = rgba_image.height();

    device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some(file_name),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[TextureFormat::Rgba8Unorm],
        },
        TextureDataOrder::LayerMajor,
        rgba_image.as_bytes(),
    )
}

fn compress(compressor: &mut BlockCompressor, device: &Device, queue: &Queue) {
    let timestamp_query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timestamp query set"),
        count: 2,
        ty: QueryType::Timestamp,
    });

    let timestamp_resolve_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("timestamp resolve buffer"),
        size: 16,
        usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC | BufferUsages::QUERY_RESOLVE,
        mapped_at_creation: false,
    });

    let timestamp_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timestamp read-back buffer"),
        size: 16,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("command encoder"),
    });

    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("compute pass"),
            timestamp_writes: Some(ComputePassTimestampWrites {
                query_set: &timestamp_query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            }),
        });

        compressor.compress(&mut pass);
    }

    encoder.resolve_query_set(&timestamp_query_set, 0..2, &timestamp_resolve_buffer, 0);

    encoder.copy_buffer_to_buffer(
        &timestamp_resolve_buffer,
        0,
        &timestamp_readback_buffer,
        0,
        16,
    );

    queue.submit([encoder.finish()]);

    {
        let buffer_slice = timestamp_readback_buffer.slice(..);

        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());

        device.poll(wgpu::Maintain::Wait);

        match rx.recv() {
            Ok(Ok(())) => {
                let data = buffer_slice.get_mapped_range();
                let timestamps: &[u64] = cast_slice(&data);

                let period = queue.get_timestamp_period() as f64;
                let start_ns = timestamps[0] as f64 * period;
                let end_ns = timestamps[1] as f64 * period;
                let duration_ms = (end_ns - start_ns) / 1_000_000.0;

                println!("Compression took: {:.3} ms", duration_ms);
            }
            _ => panic!("couldn't read from buffer"),
        }

        timestamp_readback_buffer.unmap();
    }
}

fn download_blocks_data(device: &Device, queue: &Queue, block_buffer: Buffer) -> Vec<u8> {
    let size = block_buffer.size();

    let staging_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("staging buffer"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut copy_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("copy encoder"),
    });

    copy_encoder.copy_buffer_to_buffer(&block_buffer, 0, &staging_buffer, 0, size);

    queue.submit([copy_encoder.finish()]);

    let result;

    {
        let buffer_slice = staging_buffer.slice(..);

        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(MapMode::Read, move |v| tx.send(v).unwrap());

        device.poll(Maintain::Wait);

        match rx.recv() {
            Ok(Ok(())) => {
                result = buffer_slice.get_mapped_range().to_vec();
            }
            _ => panic!("couldn't read from buffer"),
        }
    }

    staging_buffer.unmap();

    result
}

fn write_dds_file(
    file_name: &str,
    variant: CompressionVariant,
    width: u32,
    height: u32,
    block_data: Vec<u8>,
) {
    let mut dds = Dds::new_dxgi(NewDxgiParams {
        height,
        width,
        depth: None,
        format: variant.dxgi_format(),
        mipmap_levels: Some(1),
        array_layers: None,
        caps2: None,
        is_cubemap: false,
        resource_dimension: D3D10ResourceDimension::Texture2D,
        alpha_mode: AlphaMode::Straight,
    })
    .expect("failed to create DDS header");

    dds.data = block_data;

    let mut dds_name = PathBuf::from(file_name);
    dds_name.set_extension("dds");

    let mut file = File::create(dds_name).expect("failed to create output file");
    dds.write(&mut file).expect("failed to write DDS file");
}
