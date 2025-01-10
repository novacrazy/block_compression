use crate::settings::Bc6HSettings;
use crate::Bc7Settings;
use ddsfile::DxgiFormat;
use wgpu::{
    self, include_wgsl, BindGroupLayout, ComputePass, ComputePipeline, ComputePipelineDescriptor,
    Device, PipelineCompilationOptions, PipelineLayoutDescriptor,
};

#[derive(Copy, Clone)]
pub enum BlockCompressorVariant {
    BC1,
    BC2,
    BC3,
    BC4,
    BC5,
    BC6H(Bc6HSettings),
    BC7(Bc7Settings),
}

impl BlockCompressorVariant {
    pub fn entry_point(self) -> &'static str {
        match self {
            BlockCompressorVariant::BC1 => "compress_bc1",
            BlockCompressorVariant::BC2 => "compress_bc2",
            BlockCompressorVariant::BC3 => "compress_bc3",
            BlockCompressorVariant::BC4 => "compress_bc4",
            BlockCompressorVariant::BC5 => "compress_bc5",
            BlockCompressorVariant::BC6H(..) => "compress_bc6h",
            BlockCompressorVariant::BC7(..) => "compress_bc7",
        }
    }

    pub fn output_size(self, width: u32, height: u32) -> usize {
        let block_width = (width + 3) / 4;
        let block_height = (height + 3) / 4;
        let block_count = block_width * block_height;

        match self {
            BlockCompressorVariant::BC1 => (block_count * 8) as usize,
            BlockCompressorVariant::BC2 => (block_count * 16) as usize,
            BlockCompressorVariant::BC3 => (block_count * 16) as usize,
            BlockCompressorVariant::BC4 => (block_count * 8) as usize,
            BlockCompressorVariant::BC5 => (block_count * 16) as usize,
            BlockCompressorVariant::BC6H(..) => (block_count * 16) as usize,
            BlockCompressorVariant::BC7(..) => (block_count * 16) as usize,
        }
    }

    pub fn format(self) -> DxgiFormat {
        match self {
            BlockCompressorVariant::BC1 => DxgiFormat::BC1_UNorm_sRGB,
            BlockCompressorVariant::BC2 => DxgiFormat::BC2_UNorm_sRGB,
            BlockCompressorVariant::BC3 => DxgiFormat::BC3_UNorm_sRGB,
            BlockCompressorVariant::BC4 => DxgiFormat::BC4_UNorm,
            BlockCompressorVariant::BC5 => DxgiFormat::BC5_UNorm,
            BlockCompressorVariant::BC6H(..) => DxgiFormat::BC6H_UF16,
            BlockCompressorVariant::BC7(..) => DxgiFormat::BC7_UNorm_sRGB,
        }
    }
}

pub struct BlockCompressor {
    pipeline: ComputePipeline,
}

impl BlockCompressor {
    pub fn new(
        device: &Device,
        bind_group_layout: &BindGroupLayout,
        variant: BlockCompressorVariant,
    ) -> Self {
        let shader_module =
            device.create_shader_module(include_wgsl!("shader/block_compression.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("block compression pipeline layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("block compression pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some(variant.entry_point()),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline }
    }

    pub(crate) fn dispatch(&self, pass: &mut ComputePass, width: u32, height: u32) {
        pass.set_pipeline(&self.pipeline);

        let block_width = (width + 3) / 4;
        let block_height = (height + 3) / 4;

        let workgroup_width = (block_width + 7) / 8;
        let workgroup_height = (block_height + 7) / 8;

        pass.dispatch_workgroups(workgroup_width, workgroup_height, 1);
    }
}
