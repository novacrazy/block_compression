use crate::bc3_compressor::BC3Compressor;
use bytemuck::{cast_slice, Pod, Zeroable};
use ddsfile::{AlphaMode, D3D10ResourceDimension, Dds, DxgiFormat, NewDxgiParams};
use image::ImageReader;
use pollster::block_on;
use std::fs::File;
use std::num::NonZeroU64;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType,
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, ComputePassDescriptor,
    ComputePassTimestampWrites, Device, DeviceDescriptor, Dx12Compiler, Features,
    Gles3MinorVersion, Instance, InstanceDescriptor, InstanceFlags, QueryType, Queue, ShaderStages,
};

mod bc3_compressor;

#[derive(Copy, Clone, Zeroable, Pod)]
#[repr(C)]
struct SurfaceUniform {
    width: u32,
    height: u32,
    stride: u32,
    padding: [u32; 1],
}

fn main() {
    let image = ImageReader::open("test.png")
        .expect("can't open input image")
        .decode()
        .expect("can't decode image");

    let image_data = image.as_bytes();
    let width = image.width();
    let height = image.height();

    let src_size = image_data.len();
    let dst_size = src_size / 4;

    let (device, queue, dst_buffer, bind_group_layout, bind_group) =
        create_resources(image_data, width, height, src_size, dst_size);

    bc3_compression(
        width,
        height,
        &device,
        &queue,
        &bind_group_layout,
        &bind_group,
    );

    let result = read_back_data(&device, &queue, &dst_buffer, dst_size);

    write_dds_file(width, height, result);
}

fn create_resources(
    image_data: &[u8],
    width: u32,
    height: u32,
    src_size: usize,
    dst_size: usize,
) -> (Device, Queue, Buffer, BindGroupLayout, BindGroup) {
    let uniforms = SurfaceUniform {
        width,
        height,
        stride: width * 4,
        padding: Default::default(),
    };

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
            required_limits: Default::default(),
            memory_hints: Default::default(),
        },
        None,
    ))
    .expect("Failed to create device");

    let src_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("source buffer"),
        usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
        contents: image_data,
    });

    let dst_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("destination buffer"),
        size: dst_size as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    let uniforms_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("Uniforms"),
        contents: cast_slice(&[uniforms]),
        usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
    });

    let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("BC3 Bind Group Layout"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(src_size as _),
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
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size_of::<SurfaceUniform>() as _),
                },
                count: None,
            },
        ],
    });

    let bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("bind group"),
        layout: &bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: src_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: dst_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: uniforms_buffer.as_entire_binding(),
            },
        ],
    });

    (device, queue, dst_buffer, bind_group_layout, bind_group)
}

fn bc3_compression(
    width: u32,
    height: u32,
    device: &Device,
    queue: &Queue,
    bind_group_layout: &BindGroupLayout,
    bind_group: &BindGroup,
) {
    let compressor = BC3Compressor::new(device, bind_group_layout);

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

fn write_dds_file(width: u32, height: u32, result: Vec<u8>) {
    let mut dds = Dds::new_dxgi(NewDxgiParams {
        height,
        width,
        depth: None,
        format: DxgiFormat::BC3_UNorm_sRGB,
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
