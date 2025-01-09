use wgpu::{
    self, include_wgsl, BindGroupLayout, ComputePass, ComputePipeline, ComputePipelineDescriptor,
    Device, PipelineCompilationOptions, PipelineLayoutDescriptor,
};

pub struct BC3Compressor {
    pipeline: ComputePipeline,
}

impl BC3Compressor {
    pub fn new(device: &Device, bind_group_layout: &BindGroupLayout) -> Self {
        let shader = device.create_shader_module(include_wgsl!("shader/bc3.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("BC3 compressor pipeline layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("BC3 compressor pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("compress_bc3"),
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
