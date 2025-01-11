use crate::settings::{BC6HSettings, Settings};
use crate::BC7Settings;
use bytemuck::{cast_slice, Pod, Zeroable};
use ddsfile::DxgiFormat;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use wgpu::{
    self, include_wgsl, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, Buffer,
    BufferBindingType, BufferDescriptor, BufferUsages, ComputePass, ComputePipeline,
    ComputePipelineDescriptor, Device, PipelineCompilationOptions, PipelineLayoutDescriptor, Queue,
    ShaderModule, ShaderStages, TextureSampleType, TextureView, TextureViewDimension,
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

    fn block_byte_size(self) -> u32 {
        match self {
            CompressionVariant::BC1 | CompressionVariant::BC4 => 8,
            CompressionVariant::BC2
            | CompressionVariant::BC3
            | CompressionVariant::BC5
            | CompressionVariant::BC6H
            | CompressionVariant::BC7 => 16,
        }
    }

    pub fn blocks_byte_size(self, width: u32, height: u32) -> usize {
        let block_width = (width as usize + 3) / 4;
        let block_height = (height as usize + 3) / 4;
        let block_count = block_width * block_height;
        let block_size = self.block_byte_size() as usize;
        block_count * block_size
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

#[derive(Copy, Clone, Zeroable, Pod)]
#[repr(C)]
struct Uniforms {
    width: u32,
    height: u32,
    block: u32,
}

struct Task {
    variant: CompressionVariant,
    width: u32,
    height: u32,
    uniform_offset: u32,
    setting_offset: u32,
    buffer_offset: u32,
    bind_group: BindGroup,
    settings: Option<Settings>,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Status {
    Ready,
    Uploaded,
}

pub struct BlockCompressor {
    scratch_buffer: Vec<u8>,
    task: Vec<Task>,
    uniforms_buffer: Buffer,
    bc6h_settings_buffer: Buffer,
    bc7_settings_buffer: Buffer,
    bind_group_layouts: HashMap<CompressionVariant, BindGroupLayout>,
    pipelines: HashMap<CompressionVariant, ComputePipeline>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    uniforms_aligned_size: usize,
    bc6h_aligned_size: usize,
    bc7_aligned_size: usize,
    status: Status,
}

impl BlockCompressor {
    pub fn new(device: Arc<Device>, queue: Arc<Queue>) -> Self {
        let limits = device.limits();

        let alignment = limits.min_uniform_buffer_offset_alignment as usize;
        let size = size_of::<Uniforms>();
        let uniforms_aligned_size = size.div_ceil(alignment) * alignment;

        let alignment = limits.min_storage_buffer_offset_alignment as usize;
        let size = size_of::<BC6HSettings>();
        let bc6h_aligned_size = size.div_ceil(alignment) * alignment;

        let alignment = limits.min_storage_buffer_offset_alignment as usize;
        let size = size_of::<BC7Settings>();
        let bc7_aligned_size = size.div_ceil(alignment) * alignment;

        let shader_module_bc1_to_5 =
            device.create_shader_module(include_wgsl!("shader/BC1_to_5.wgsl"));
        let shader_module_bc6h = device.create_shader_module(include_wgsl!("shader/BC6H.wgsl"));
        let shader_module_bc7 = device.create_shader_module(include_wgsl!("shader/BC7.wgsl"));

        let uniforms_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("uniforms"),
            size: (uniforms_aligned_size * 16) as _,
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let bc6h_settings_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("bc6h settings"),
            size: (bc6h_aligned_size * 16) as _,
            usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let bc7_settings_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("bc7 settings"),
            size: (bc7_aligned_size * 16) as _,
            usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
            mapped_at_creation: false,
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

        Self {
            scratch_buffer: Vec::default(),
            task: Vec::default(),
            uniforms_buffer,
            bc6h_settings_buffer,
            bc7_settings_buffer,
            bind_group_layouts,
            pipelines,
            device,
            queue,
            uniforms_aligned_size,
            bc6h_aligned_size,
            bc7_aligned_size,
            status: Status::Ready,
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
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            },
        ];

        match variant {
            CompressionVariant::BC6H => {
                layout_entries.push(BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(size_of::<BC6HSettings>() as _),
                    },
                    count: None,
                });
            }
            CompressionVariant::BC7 => {
                layout_entries.push(BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(size_of::<BC7Settings>() as _),
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

    /// Add a task to compress the given texture view to the given block compression variant.
    ///
    /// BC6H adn BC7 can be configured with the [`BC6HSettings`] and [`BC7Settings`] structs.
    /// In case settings are not provided for them, their respective `slow` settings will be
    /// used as a default.
    #[allow(clippy::too_many_arguments)]
    pub fn add_compression_task(
        &mut self,
        variant: CompressionVariant,
        texture_view: &TextureView,
        width: u32,
        height: u32,
        buffer: &Buffer,
        offset: Option<u32>,
        settings: impl Into<Option<Settings>>,
    ) {
        assert_ne!(
            self.status,
            Status::Uploaded,
            "added compression task after upload and before dispatch"
        );

        let mut settings = settings.into();

        if variant == CompressionVariant::BC6H && settings.is_none() {
            settings = Some(Settings::BC6H(BC6HSettings::slow()));
        }

        if variant == CompressionVariant::BC7 && settings.is_none() {
            settings = Some(Settings::BC7(BC7Settings::alpha_slow()));
        }

        self.add_task(
            variant,
            texture_view,
            width,
            height,
            buffer,
            offset,
            settings,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_task(
        &mut self,
        variant: CompressionVariant,
        texture_view: &TextureView,
        width: u32,
        height: u32,
        buffer: &Buffer,
        offset: Option<u32>,
        settings: Option<Settings>,
    ) {
        assert_eq!(height % 4, 0);
        assert_eq!(width % 4, 0);

        let block_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("block buffer"),
            size: variant.blocks_byte_size(width, height) as u64,
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
                        resource: buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms_buffer.as_entire_binding(),
                    },
                ],
            }),
            CompressionVariant::BC6H => self.device.create_bind_group(&BindGroupDescriptor {
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
                    BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: self.bc6h_settings_buffer.as_entire_binding(),
                    },
                ],
            }),
            CompressionVariant::BC7 => self.device.create_bind_group(&BindGroupDescriptor {
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
                    BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: self.bc7_settings_buffer.as_entire_binding(),
                    },
                ],
            }),
        };

        self.task.push(Task {
            variant,
            width,
            height,
            uniform_offset: 0,
            setting_offset: 0,
            buffer_offset: offset.unwrap_or(0),
            bind_group,
            settings,
        });
    }

    pub fn upload(&mut self) {
        if self.status == Status::Uploaded {
            return;
        }
        self.status = Status::Uploaded;

        self.update_buffer_sizes();

        self.scratch_buffer.clear();
        for (index, task) in self
            .task
            .iter_mut()
            .filter(|task| task.variant == CompressionVariant::BC6H)
            .enumerate()
        {
            let offset = index * self.bc6h_aligned_size;
            task.setting_offset = offset as u32;

            match &task.settings {
                Some(Settings::BC6H(settings)) => {
                    self.scratch_buffer
                        .resize(offset + self.bc6h_aligned_size, 0);
                    self.scratch_buffer[offset..offset + size_of::<BC6HSettings>()]
                        .copy_from_slice(cast_slice(&[*settings]));
                }
                _ => panic!("internal error: No BC6H setting found"),
            }
        }
        if !self.scratch_buffer.is_empty() {
            if let Some(mut data) = self.queue.write_buffer_with(
                &self.bc6h_settings_buffer,
                0,
                NonZeroU64::new(self.scratch_buffer.len() as u64).unwrap(),
            ) {
                data.copy_from_slice(&self.scratch_buffer);
            }
        }

        self.scratch_buffer.clear();
        for (index, task) in self
            .task
            .iter_mut()
            .filter(|task| task.variant == CompressionVariant::BC7)
            .enumerate()
        {
            let offset = index * self.bc7_aligned_size;
            task.setting_offset = offset as u32;

            match &task.settings {
                Some(Settings::BC7(settings)) => {
                    self.scratch_buffer
                        .resize(offset + self.bc7_aligned_size, 0);
                    self.scratch_buffer[offset..offset + size_of::<BC7Settings>()]
                        .copy_from_slice(cast_slice(&[*settings]));
                }
                _ => panic!("internal error: No BC7 setting found"),
            }
        }
        if !self.scratch_buffer.is_empty() {
            if let Some(mut data) = self.queue.write_buffer_with(
                &self.bc6h_settings_buffer,
                0,
                NonZeroU64::new(self.scratch_buffer.len() as u64).unwrap(),
            ) {
                data.copy_from_slice(&self.scratch_buffer);
            }
        }

        self.scratch_buffer.clear();
        for (index, task) in self.task.iter_mut().enumerate() {
            let offset = index * self.uniforms_aligned_size;
            task.uniform_offset = offset as u32;

            let uniforms = Uniforms {
                width: task.width,
                height: task.height,
                block: task.buffer_offset,
            };

            self.scratch_buffer
                .resize(offset + self.uniforms_aligned_size, 0);
            self.scratch_buffer[offset..offset + size_of::<Uniforms>()]
                .copy_from_slice(cast_slice(&[uniforms]));
        }
        if !self.scratch_buffer.is_empty() {
            if let Some(mut data) = self.queue.write_buffer_with(
                &self.uniforms_buffer,
                0,
                NonZeroU64::new(self.scratch_buffer.len() as u64).unwrap(),
            ) {
                data.copy_from_slice(&self.scratch_buffer);
            }
        }
    }

    fn update_buffer_sizes(&mut self) {
        let total_uniforms_size = self.uniforms_aligned_size * self.task.len();
        if total_uniforms_size > self.uniforms_buffer.size() as usize {
            self.uniforms_buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("uniforms buffer"),
                size: total_uniforms_size as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });
        }

        let bc6_setting_count = self
            .task
            .iter()
            .filter(|task| task.variant == CompressionVariant::BC6H)
            .count();

        let total_bc6h_size = self.bc6h_aligned_size * bc6_setting_count;
        if total_bc6h_size > self.bc6h_settings_buffer.size() as usize {
            self.bc6h_settings_buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("bc6h settings buffer"),
                size: total_bc6h_size as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }

        let bc7_setting_count = self
            .task
            .iter()
            .filter(|task| task.variant == CompressionVariant::BC7)
            .count();

        let total_bc7_size = self.bc7_aligned_size * bc7_setting_count;
        if total_bc7_size > self.bc7_settings_buffer.size() as usize {
            self.bc7_settings_buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("bc7 settings buffer"),
                size: total_bc7_size as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
    }

    pub fn compress(&mut self, pass: &mut ComputePass) {
        assert_eq!(
            self.status,
            Status::Uploaded,
            "dispatch called before upload"
        );
        self.status = Status::Ready;

        for task in self.task.drain(..) {
            let pipeline = self
                .pipelines
                .get(&task.variant)
                .expect("can't find pipeline for variant");

            pass.set_pipeline(pipeline);

            match task.variant {
                CompressionVariant::BC6H | CompressionVariant::BC7 => {
                    pass.set_bind_group(
                        0,
                        &task.bind_group,
                        &[task.uniform_offset, task.setting_offset],
                    );
                }
                _ => {
                    pass.set_bind_group(0, &task.bind_group, &[task.uniform_offset]);
                }
            }

            let block_width = (task.width + 3) / 4;
            let block_height = (task.height + 3) / 4;

            let workgroup_width = (block_width + 7) / 8;
            let workgroup_height = (block_height + 7) / 8;

            pass.dispatch_workgroups(workgroup_width, workgroup_height, 1);
        }
    }
}
