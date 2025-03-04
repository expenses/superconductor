pub struct BindGroupLayouts {
    pub uniform: wgpu::BindGroupLayout,
    pub model: wgpu::BindGroupLayout,
    pub tonemap: wgpu::BindGroupLayout,
    pub uint_texture: wgpu::BindGroupLayout,
    pub sampled_texture: wgpu::BindGroupLayout,
    pub joints: wgpu::BindGroupLayout,
}

impl BindGroupLayouts {
    pub fn new(device: &wgpu::Device, options: &crate::pipelines::PipelineOptions) -> Self {
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            count: None,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        };

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        };

        let d2array_texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
        };

        let texture_array_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
        };

        let cubemap_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::Cube,
                multisampled: false,
            },
        };

        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        };

        let uint_texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        };

        Self {
            uniform: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("uniform bind group layout"),
                entries: &[
                    uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                    sampler_entry(1),
                    cubemap_entry(2),
                    d2array_texture_entry(3),
                    d2array_texture_entry(4),
                    d2array_texture_entry(5),
                    d2array_texture_entry(6),
                    texture_entry(7),
                    texture_entry(8),
                    texture_entry(9),
                    texture_entry(10),
                    texture_entry(11),
                    texture_entry(12),
                    texture_entry(13),
                ],
            }),
            model: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("model bind group layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    texture_entry(2),
                    texture_entry(3),
                    uniform_entry(4, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                    sampler_entry(5),
                ],
            }),
            tonemap: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mirror bind group layout"),
                entries: &[
                    sampler_entry(0),
                    if options.multiview.is_none() {
                        texture_entry(1)
                    } else {
                        texture_array_entry(1)
                    },
                ],
            }),
            uint_texture: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("uint texture bind group layout"),
                entries: &[uint_texture_entry(0)],
            }),
            sampled_texture: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sampled texture bind group layout"),
                entries: &[sampler_entry(0), texture_entry(1)],
            }),
            joints: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("joints bind group layout"),
                entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
            }),
        }
    }
}
