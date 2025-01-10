use crate::settings::Bc6HSettings;
use crate::Bc7Settings;
use bytemuck::cast_slice;
use ddsfile::DxgiFormat;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use wgpu::{
    self, include_wgsl, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, Buffer,
    BufferBinding, BufferBindingType, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
    ComputePass, ComputePipeline, ComputePipelineDescriptor, Device, Extent3d, ImageCopyBuffer,
    ImageCopyTexture, ImageDataLayout, Maintain, MapMode, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, Queue, ShaderModule, ShaderStages, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDimension,
};

#[derive(Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum CompressionVariant {
    BC1,
    BC2,
    BC3,
    BC4,
    BC5,
    BC6H,
    BC7,
}

impl CompressionVariant {
    fn name(self) -> &'static str {
        match self {
            CompressionVariant::BC1 => "bc1",
            CompressionVariant::BC2 => "bc2",
            CompressionVariant::BC3 => "bc3",
            CompressionVariant::BC4 => "bc4",
            CompressionVariant::BC5 => "bc5",
            CompressionVariant::BC6H => "bc6h",
            CompressionVariant::BC7 => "bc7",
        }
    }

    fn entry_point(self) -> &'static str {
        match self {
            CompressionVariant::BC1 => "compress_bc1",
            CompressionVariant::BC2 => "compress_bc2",
            CompressionVariant::BC3 => "compress_bc3",
            CompressionVariant::BC4 => "compress_bc4",
            CompressionVariant::BC5 => "compress_bc5",
            CompressionVariant::BC6H => "compress_bc6h",
            CompressionVariant::BC7 => "compress_bc7",
        }
    }

    fn block_size(self) -> u32 {
        match self {
            CompressionVariant::BC1 | CompressionVariant::BC4 => 8,
            CompressionVariant::BC2
            | CompressionVariant::BC3
            | CompressionVariant::BC5
            | CompressionVariant::BC6H
            | CompressionVariant::BC7 => 16,
        }
    }

    fn output_size(self, width: u32, height: u32) -> usize {
        let block_width = (width as usize + 3) / 4;
        let block_height = (height as usize + 3) / 4;
        let block_count = block_width * block_height;
        let block_size = self.block_size() as usize;
        block_count * block_size
    }

    pub(crate) fn texture_format(self) -> TextureFormat {
        match self {
            CompressionVariant::BC1 => TextureFormat::Bc1RgbaUnormSrgb,
            CompressionVariant::BC2 => TextureFormat::Bc2RgbaUnormSrgb,
            CompressionVariant::BC3 => TextureFormat::Bc3RgbaUnormSrgb,
            CompressionVariant::BC4 => TextureFormat::Bc4RUnorm,
            CompressionVariant::BC5 => TextureFormat::Bc5RgUnorm,
            CompressionVariant::BC6H => TextureFormat::Bc6hRgbUfloat,
            CompressionVariant::BC7 => TextureFormat::Bc7RgbaUnormSrgb,
        }
    }

    // TODO: NHA move behind feature flag.
    pub(crate) fn dxgi_format(self) -> DxgiFormat {
        match self {
            CompressionVariant::BC1 => DxgiFormat::BC1_UNorm_sRGB,
            CompressionVariant::BC2 => DxgiFormat::BC2_UNorm_sRGB,
            CompressionVariant::BC3 => DxgiFormat::BC3_UNorm_sRGB,
            CompressionVariant::BC4 => DxgiFormat::BC4_UNorm,
            CompressionVariant::BC5 => DxgiFormat::BC5_UNorm,
            CompressionVariant::BC6H => DxgiFormat::BC6H_UF16,
            CompressionVariant::BC7 => DxgiFormat::BC7_UNorm_sRGB,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum TaskStatus {
    Created,
    Dispatched,
}

struct Task {
    key: String,
    dispatched: TaskStatus,
    variant: CompressionVariant,
    width: u32,
    height: u32,
    settings_offset: Option<usize>,
    block_buffer: Buffer,
    bind_group: BindGroup,
}

pub struct BlockCompressor {
    scratch_buffer: Vec<u8>,
    task: Vec<Task>,
    bc6h_settings: Vec<Bc6HSettings>,
    bc7_settings: Vec<Bc7Settings>,
    bc6h_settings_buffer: Buffer,
    bc7_settings_buffer: Buffer,
    bind_group_layouts: HashMap<CompressionVariant, BindGroupLayout>,
    pipelines: HashMap<CompressionVariant, ComputePipeline>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    bc6h_aligned_size: usize,
    bc7_aligned_size: usize,
}

impl BlockCompressor {
    pub fn new(device: Arc<Device>, queue: Arc<Queue>) -> Self {
        let shader_module_bc1_to_5 =
            device.create_shader_module(include_wgsl!("shader/BC1_to_5.wgsl"));
        let shader_module_bc6h = device.create_shader_module(include_wgsl!("shader/BC6H.wgsl"));
        let shader_module_bc7 = device.create_shader_module(include_wgsl!("shader/BC7.wgsl"));

        let bc7_settings_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bc7 settings"),
            contents: cast_slice(&[Bc7Settings::alpha_very_fast()]),
            usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
        });

        let bc6h_settings_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bc6h settings"),
            contents: cast_slice(&[Bc6HSettings::very_fast()]),
            usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
        });

        let mut bind_group_layouts = HashMap::new();
        let mut pipelines = HashMap::new();

        Self::create_pipeline(
            &device,
            &shader_module_bc1_to_5,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC1,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc1_to_5,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC2,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc1_to_5,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC3,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc1_to_5,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC4,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc1_to_5,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC5,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc6h,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC6H,
        );
        Self::create_pipeline(
            &device,
            &shader_module_bc7,
            &mut bind_group_layouts,
            &mut pipelines,
            CompressionVariant::BC7,
        );

        let limits = device.limits();

        let alignment = limits.min_storage_buffer_offset_alignment as usize;
        let size = size_of::<Bc6HSettings>();
        let bc6h_aligned_size = size.div_ceil(alignment) * alignment;

        let alignment = limits.min_storage_buffer_offset_alignment as usize;
        let size = size_of::<Bc7Settings>();
        let bc7_aligned_size = size.div_ceil(alignment) * alignment;

        Self {
            scratch_buffer: Vec::default(),
            task: Vec::default(),
            bc6h_settings: Vec::default(),
            bc7_settings: Vec::default(),
            bc6h_settings_buffer,
            bc7_settings_buffer,
            bind_group_layouts,
            pipelines,
            device,
            queue,
            bc6h_aligned_size,
            bc7_aligned_size,
        }
    }

    fn create_pipeline(
        device: &Device,
        shader_module: &ShaderModule,
        bind_group_layouts: &mut HashMap<CompressionVariant, BindGroupLayout>,
        pipelines: &mut HashMap<CompressionVariant, ComputePipeline>,
        variant: CompressionVariant,
    ) {
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
                    min_binding_size: None,
                },
                count: None,
            },
        ];

        match variant {
            CompressionVariant::BC6H => {
                layout_entries.push(BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(size_of::<Bc6HSettings>() as _),
                    },
                    count: None,
                });
            }
            CompressionVariant::BC7 => {
                layout_entries.push(BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(size_of::<Bc7Settings>() as _),
                    },
                    count: None,
                });
            }
            _ => {}
        }

        let name = variant.name();

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some(&format!("{name} bind group layout")),
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some(&format!("{name} block compression pipeline layout")),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some(&format!("{name} block compression pipeline")),
            layout: Some(&pipeline_layout),
            module: shader_module,
            entry_point: Some(variant.entry_point()),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });

        bind_group_layouts.insert(variant, bind_group_layout);
        pipelines.insert(variant, pipeline);
    }

    pub fn add_bc1_task(&mut self, key: &str, texture_view: &TextureView, width: u32, height: u32) {
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC1,
            None,
        );
    }

    pub fn add_bc2_task(&mut self, key: &str, texture_view: &TextureView, width: u32, height: u32) {
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC2,
            None,
        );
    }

    pub fn add_bc3_task(&mut self, key: &str, texture_view: &TextureView, width: u32, height: u32) {
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC3,
            None,
        );
    }

    pub fn add_bc4_task(&mut self, key: &str, texture_view: &TextureView, width: u32, height: u32) {
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC4,
            None,
        );
    }

    pub fn add_bc5_task(&mut self, key: &str, texture_view: &TextureView, width: u32, height: u32) {
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC5,
            None,
        );
    }

    pub fn add_bc6h_task(
        &mut self,
        key: &str,
        texture_view: &TextureView,
        width: u32,
        height: u32,
        settings: Bc6HSettings,
    ) {
        let settings_offset = self.bc6h_settings.len() * self.bc6h_aligned_size;
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC6H,
            Some(settings_offset),
        );
        self.bc6h_settings.push(settings);
    }

    pub fn add_bc7_task(
        &mut self,
        key: &str,
        texture_view: &TextureView,
        width: u32,
        height: u32,
        settings: Bc7Settings,
    ) {
        let settings_offset = self.bc7_settings.len() * self.bc7_aligned_size;
        self.add_task(
            key,
            texture_view,
            width,
            height,
            CompressionVariant::BC7,
            Some(settings_offset),
        );
        self.bc7_settings.push(settings);
    }

    fn add_task(
        &mut self,
        key: &str,
        texture_view: &TextureView,
        width: u32,
        height: u32,
        variant: CompressionVariant,
        settings_offset: Option<usize>,
    ) {
        assert_eq!(height % 4, 0);
        assert_eq!(width % 4, 0);

        let block_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("block buffer"),
            size: variant.output_size(width, height) as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC | BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let bind_group_layout = self
            .bind_group_layouts
            .get(&variant)
            .expect("Can't find bind group layout for variant");

        let bind_group = match variant {
            CompressionVariant::BC1
            | CompressionVariant::BC2
            | CompressionVariant::BC3
            | CompressionVariant::BC4
            | CompressionVariant::BC5 => self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(texture_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: block_buffer.as_entire_binding(),
                    },
                ],
            }),
            CompressionVariant::BC6H => self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&texture_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: block_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &self.bc6h_settings_buffer,
                            offset: 0,
                            size: None,
                        }),
                    },
                ],
            }),
            CompressionVariant::BC7 => self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("bind group"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&texture_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: block_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &self.bc7_settings_buffer,
                            offset: 0,
                            size: None,
                        }),
                    },
                ],
            }),
        };

        self.task.push(Task {
            key: key.to_string(),
            dispatched: TaskStatus::Created,
            variant,
            width,
            height,
            settings_offset,
            block_buffer,
            bind_group,
        });
    }

    pub fn upload(&mut self) {
        if !self.bc6h_settings.is_empty() {
            let total_bc6h_size = self.bc6h_aligned_size * self.bc6h_settings.len();
            if total_bc6h_size > self.bc6h_settings_buffer.size() as usize {
                self.bc6h_settings_buffer = self.device.create_buffer(&BufferDescriptor {
                    label: Some("bc6h settings buffer"),
                    size: total_bc6h_size as u64,
                    usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
            }

            self.scratch_buffer.clear();
            for (i, settings) in self.bc6h_settings.iter().enumerate() {
                let offset = i * self.bc6h_aligned_size;
                self.scratch_buffer
                    .resize(offset + self.bc6h_aligned_size, 0);
                self.scratch_buffer[offset..offset + size_of::<Bc6HSettings>()]
                    .copy_from_slice(cast_slice(&[*settings]));
            }
            if let Some(mut data) = self.queue.write_buffer_with(
                &self.bc6h_settings_buffer,
                0,
                NonZeroU64::new(self.scratch_buffer.len() as u64).unwrap(),
            ) {
                data.copy_from_slice(&self.scratch_buffer);
            }
        }

        if !self.bc7_settings.is_empty() {
            let total_bc7_size = self.bc7_aligned_size * self.bc7_settings.len();
            if total_bc7_size > self.bc7_settings_buffer.size() as usize {
                self.bc7_settings_buffer = self.device.create_buffer(&BufferDescriptor {
                    label: Some("bc7 settings buffer"),
                    size: total_bc7_size as u64,
                    usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
            }

            self.scratch_buffer.clear();
            for (i, settings) in self.bc7_settings.iter().enumerate() {
                let offset = i * self.bc7_aligned_size;
                self.scratch_buffer
                    .resize(offset + self.bc7_aligned_size, 0);
                self.scratch_buffer[offset..offset + size_of::<Bc7Settings>()]
                    .copy_from_slice(cast_slice(&[*settings]));
            }
            if let Some(mut data) = self.queue.write_buffer_with(
                &self.bc7_settings_buffer,
                0,
                NonZeroU64::new(self.scratch_buffer.len() as u64).unwrap(),
            ) {
                data.copy_from_slice(&self.scratch_buffer);
            }
        }

        self.bc6h_settings.clear();
        self.bc7_settings.clear();
    }

    pub fn compress(&mut self, pass: &mut ComputePass) {
        assert!(
            self.bc6h_settings.is_empty(),
            "dispatch called before upload"
        );
        assert!(
            self.bc7_settings.is_empty(),
            "dispatch called before upload"
        );

        for task in self
            .task
            .iter_mut()
            .filter(|task| task.dispatched == TaskStatus::Created)
        {
            task.dispatched = TaskStatus::Dispatched;

            let pipeline = self
                .pipelines
                .get(&task.variant)
                .expect("can't find pipeline for variant");

            pass.set_pipeline(pipeline);

            match task.settings_offset {
                None => {
                    pass.set_bind_group(0, &task.bind_group, &[]);
                }
                Some(offset) => {
                    pass.set_bind_group(0, &task.bind_group, &[0, 0, offset as _]);
                }
            }

            let block_width = (task.width + 3) / 4;
            let block_height = (task.height + 3) / 4;

            let workgroup_width = (block_width + 7) / 8;
            let workgroup_height = (block_height + 7) / 8;

            pass.dispatch_workgroups(workgroup_width, workgroup_height, 1);
        }
    }

    pub fn get_texture(&mut self, name: &str) -> Option<Texture> {
        let task_index = self.task.iter().position(|task| task.key == name)?;
        let task = self.task.swap_remove(task_index);

        let block_width = (task.width + 3) / 4;
        let block_height = (task.height + 3) / 4;

        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("compressed texture"),
            size: Extent3d {
                width: task.width,
                height: task.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: task.variant.texture_format(),
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("copy encoder"),
            });

        let bytes_per_row = block_width * task.variant.block_size();
        assert_eq!(
            bytes_per_row % 256,
            0,
            "bytes per row is not a multiple of 256"
        );

        encoder.copy_buffer_to_texture(
            ImageCopyBuffer {
                buffer: &task.block_buffer,
                layout: ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(block_height),
                },
            },
            ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: task.width,
                height: task.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit([encoder.finish()]);

        Some(texture)
    }

    pub fn get_block_data(&mut self, key: &str) -> Option<Vec<u8>> {
        let task_index = self.task.iter().position(|task| task.key == key)?;
        let task = self.task.swap_remove(task_index);

        let size = task.block_buffer.size();

        let staging_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("staging buffer"),
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut copy_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("copy encoder"),
            });

        copy_encoder.copy_buffer_to_buffer(&task.block_buffer, 0, &staging_buffer, 0, size);

        self.queue.submit([copy_encoder.finish()]);

        let result;

        {
            let buffer_slice = staging_buffer.slice(..);

            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(MapMode::Read, move |v| tx.send(v).unwrap());

            self.device.poll(Maintain::Wait);

            match rx.recv() {
                Ok(Ok(())) => {
                    result = buffer_slice.get_mapped_range().to_vec();
                }
                _ => panic!("couldn't read from buffer"),
            }
        }

        staging_buffer.unmap();

        Some(result)
    }
}
