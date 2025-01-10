use crate::block_compressor::{BlockCompressor, BlockCompressorVariant};
use bytemuck::cast_slice;
use ddsfile::{AlphaMode, D3D10ResourceDimension, Dds, NewDxgiParams};
use image::{EncodableLayout, ImageReader};
use pollster::block_on;
use std::fs::File;
use std::num::NonZeroU64;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, Buffer,
    BufferBindingType, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
    ComputePassDescriptor, ComputePassTimestampWrites, Device, DeviceDescriptor, Dx12Compiler,
    Extent3d, Features, Gles3MinorVersion, ImageCopyTexture, ImageDataLayout, Instance,
    InstanceDescriptor, InstanceFlags, Limits, MemoryHints, Origin3d, QueryType, Queue,
    ShaderStages, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension,
};

mod block_compressor;
mod settings;

pub use settings::Bc7Settings;

fn main() {
    let variant = BlockCompressorVariant::BC7(Bc7Settings::alpha_ultrafast());

    let image = ImageReader::open("input4096alpha.png")
        .expect("can't open input image")
        .decode()
        .expect("can't decode image");

    let rgba_image = image.to_rgba8();
    let width = rgba_image.width();
    let height = rgba_image.height();
    let image_data = rgba_image.as_bytes();
    let dst_size = variant.output_size(width, height);

    let (device, queue, dst_buffer, bind_group_layout, bind_group) =
        create_resources(variant, width, height, image_data, dst_size);

    bc_compression(
        &device,
        &queue,
        &bind_group_layout,
        &bind_group,
        variant,
        width,
        height,
    );

    let result = read_back_data(&device, &queue, &dst_buffer, dst_size);

    write_dds_file(variant, width, height, result);
}

fn create_resources(
    variant: BlockCompressorVariant,
    width: u32,
    height: u32,
    image_data: &[u8],
    dst_size: usize,
) -> (Device, Queue, Buffer, BindGroupLayout, BindGroup) {
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
            required_features: Features::TIMESTAMP_QUERY,
            required_limits: Limits::default(),
            memory_hints: MemoryHints::default(),
        },
        None,
    ))
    .expect("Failed to create device");

    let src_texture = device.create_texture(&TextureDescriptor {
        label: Some("source texture"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[TextureFormat::Rgba8Unorm],
    });

    queue.write_texture(
        ImageCopyTexture {
            texture: &src_texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        image_data,
        ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    let dst_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("destination buffer"),
        size: dst_size as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let mut layout_entries = vec![
        BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(dst_size as _),
            },
            count: None,
        },
    ];

    match variant {
        BlockCompressorVariant::BC6H(settings) => {
            let bc6h_settings_buffer = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("bc6h settings"),
                contents: cast_slice(&[settings]),
                usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
            });

            layout_entries.push(BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size_of::<Bc7Settings>() as _),
                },
                count: None,
            });

            let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("bind group layout"),
                entries: &layout_entries,
            });

            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&src_texture.create_view(
                            &TextureViewDescriptor {
                                format: Some(TextureFormat::Rgba8Unorm),
                                ..Default::default()
                            },
                        )),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: dst_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: bc6h_settings_buffer.as_entire_binding(),
                    },
                ],
            });

            (device, queue, dst_buffer, bind_group_layout, bind_group)
        }
        BlockCompressorVariant::BC7(settings) => {
            let bc7_settings_buffer = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("bc7 settings"),
                contents: cast_slice(&[settings]),
                usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
            });

            layout_entries.push(BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size_of::<Bc7Settings>() as _),
                },
                count: None,
            });

            let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("bind group layout"),
                entries: &layout_entries,
            });

            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&src_texture.create_view(
                            &TextureViewDescriptor {
                                format: Some(TextureFormat::Rgba8Unorm),
                                ..Default::default()
                            },
                        )),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: dst_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: bc7_settings_buffer.as_entire_binding(),
                    },
                ],
            });

            (device, queue, dst_buffer, bind_group_layout, bind_group)
        }
        _ => {
            let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("bind group layout"),
                entries: &layout_entries,
            });

            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&src_texture.create_view(
                            &TextureViewDescriptor {
                                format: Some(TextureFormat::Rgba8Unorm),
                                ..Default::default()
                            },
                        )),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: dst_buffer.as_entire_binding(),
                    },
                ],
            });

            (device, queue, dst_buffer, bind_group_layout, bind_group)
        }
    }
}

fn bc_compression(
    device: &Device,
    queue: &Queue,
    bind_group_layout: &BindGroupLayout,
    bind_group: &BindGroup,
    variant: BlockCompressorVariant,
    width: u32,
    height: u32,
) {
    let compressor = BlockCompressor::new(device, bind_group_layout, variant);

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

        pass.set_bind_group(0, bind_group, &[]);
        compressor.dispatch(&mut pass, width, height);
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

fn read_back_data(
    device: &Device,
    queue: &Queue,
    block_buffer: &Buffer,
    dst_size: usize,
) -> Vec<u8> {
    let staging_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("staging buffer"),
        size: dst_size as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut copy_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("copy encoder"),
    });

    copy_encoder.copy_buffer_to_buffer(block_buffer, 0, &staging_buffer, 0, dst_size as u64);

    queue.submit([copy_encoder.finish()]);

    let result;

    {
        let buffer_slice = staging_buffer.slice(..);

        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());

        device.poll(wgpu::Maintain::Wait);

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

fn write_dds_file(variant: BlockCompressorVariant, width: u32, height: u32, result: Vec<u8>) {
    let mut dds = Dds::new_dxgi(NewDxgiParams {
        height,
        width,
        depth: None,
        format: variant.format(),
        mipmap_levels: Some(1),
        array_layers: None,
        caps2: None,
        is_cubemap: false,
        resource_dimension: D3D10ResourceDimension::Texture2D,
        alpha_mode: AlphaMode::Straight,
    })
    .expect("failed to create DDS header");

    dds.data = result;

    let mut file = File::create("output.dds").expect("failed to create output file");
    dds.write(&mut file).expect("failed to write DDS file");
}
