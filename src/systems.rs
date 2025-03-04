use crate::components::{
    AnimatedModel, AnimatedModelUrl, AnimationJoints, AnimationState, Instance, InstanceOf,
    InstanceRanges, Instances, JointBuffer, JointBuffers, JointsOffset, Model, ModelUrl,
    PendingAnimatedModel, PendingModel,
};
use crate::resources::{
    AnimatedVertexBuffers, BindGroupLayouts, BoundingSphereParams, Camera, CompositeBindGroup,
    CullingParams, Device, HttpClient, IndexBuffer, InstanceBuffer, IntermediateColorFramebuffer,
    IntermediateDepthFramebuffer, LineBuffer, MainBindGroup, NewIblCubemap, NewLightvolTextures,
    ParticleBuffer, PipelineOptions, Pipelines, ProbesArrayInfo, Queue, SurfaceFrameView,
    TextureSettings, UniformBuffer, VertexBuffers,
};
use bevy_ecs::prelude::{Added, Commands, Entity, Local, Query, Res, ResMut, Without};
use renderer_core::{
    arc_swap::ArcSwapOption,
    assets, bytemuck,
    culling::{BoundingSphereCullingParams, CullingFrustum},
    glam::Mat4,
    shared_structs::{self, Settings},
    spawn, GpuInstance, MutableBindGroup, Texture,
};
use std::sync::{Arc, atomic::Ordering};
use wgpu::util::DeviceExt;

pub(crate) mod debugging;
pub(crate) mod rendering;

// todo: probably merge all the setup systems or move them into the main code.
pub(crate) fn create_bind_group_layouts_and_pipelines(
    device: Res<Device>,
    pipeline_options: Res<PipelineOptions>,
    mut commands: Commands,
) {
    let device = &device.0;

    let bind_group_layouts = renderer_core::BindGroupLayouts::new(device, &pipeline_options.0);

    let pipelines = renderer_core::Pipelines::new(device, &bind_group_layouts, &pipeline_options.0);

    commands.insert_resource(BindGroupLayouts(Arc::new(bind_group_layouts)));
    commands.insert_resource(Pipelines(Arc::new(pipelines)));
    commands.insert_resource(IntermediateColorFramebuffer(Default::default()));
    commands.insert_resource(IntermediateDepthFramebuffer(Default::default()));
    commands.insert_resource(CompositeBindGroup(None));
}

pub(crate) fn clear_instance_buffers(
    mut instance_buffer: ResMut<InstanceBuffer>,
    mut query: Query<&mut Instances>,
) {
    instance_buffer.0.clear();

    query.for_each_mut(|mut instances| instances.clear());
}

pub(crate) fn clear_joint_buffers(mut query: Query<&mut JointBuffers>) {
    query.for_each_mut(|mut joint_buffers| {
        joint_buffers.next_buffer = 0;

        for joint_buffer in &mut joint_buffers.buffers {
            joint_buffer.staging.clear();
        }
    })
}

pub(crate) fn clear_line_buffer(mut line_buffer: ResMut<LineBuffer>) {
    line_buffer.staging.clear();
    line_buffer.buffer.clear();
}

pub(crate) fn clear_particle_buffer(mut particle_buffer: ResMut<ParticleBuffer>) {
    particle_buffer.staging.clear();
    particle_buffer.buffer.clear();
}

pub(crate) fn progress_animation_times(
    mut instance_query: Query<(&InstanceOf, &mut AnimationState)>,
    model_query: Query<&AnimatedModel>,
    mut times_error_reported: Local<u32>,
) {
    instance_query.for_each_mut(|(instance_of, mut animation_state)| {
        match model_query.get(instance_of.0) {
            Ok(animated_model) => {
                let animations = &animated_model
                .0
                .animation_data
                .animations;

                if let Some(animation) = animations
                    .get(animation_state.animation_index)
                {
                    animation_state.time =
                        (animation_state.time + 1.0 / 60.0) % animation.total_time();
                } else {
                    log::warn!("Got an error when progressing animations: animation index {} is out of range of {} animations", animation_state.animation_index, animations.len());
                }
            }
            Err(error) => {
                // todo: this is very messy.
                if *times_error_reported < 5 {
                    log::warn!("Got an error when progressing animations: {}", error);
                    *times_error_reported += 1;
                }
            }
        }
    })
}

pub(crate) fn sample_animations(
    mut instance_query: Query<(&InstanceOf, &mut AnimationJoints, &AnimationState)>,
    model_query: Query<&AnimatedModel>,
) {
    instance_query.for_each_mut(|(instance_of, mut animation_joints, animation_state)| {
        match model_query.get(instance_of.0) {
            Ok(animated_model) => {
                let animations = &animated_model.0.animation_data.animations;

                if let Some(animation) = animations.get(animation_state.animation_index) {
                    animation.animate(&mut animation_joints.0, animation_state.time);
                }
            }
            Err(error) => {
                log::warn!("Got an error when sampling animations: {}", error);
            }
        }
    })
}

pub(crate) fn upload_joint_buffers(query: Query<&JointBuffers>, queue: Res<Queue>) {
    query.for_each(|joint_buffers| {
        for joint_buffer in &joint_buffers.buffers[..joint_buffers.next_buffer + 1] {
            queue.0.write_buffer(
                &joint_buffer.buffer,
                0,
                bytemuck::cast_slice(&joint_buffer.staging),
            );
        }
    })
}

pub(crate) fn push_joints(
    mut instance_query: Query<(Entity, &InstanceOf, &mut AnimationJoints)>,
    mut model_query: Query<(&AnimatedModel, &mut JointBuffers)>,
    device: Res<Device>,
    bind_group_layouts: Res<BindGroupLayouts>,
    mut commands: Commands,
) {
    instance_query.for_each_mut(|(entity, instance_of, mut animation_joints)| {
        match model_query.get_mut(instance_of.0) {
            Ok((animated_model, mut joint_buffers)) => {
                if joint_buffers.buffers[joint_buffers.next_buffer]
                    .staging
                    .remaining_capacity()
                    < animated_model.0.num_joints() as usize
                {
                    joint_buffers.next_buffer += 1;

                    if joint_buffers.next_buffer >= joint_buffers.buffers.len() {
                        joint_buffers
                            .buffers
                            .push(JointBuffer::new(&device.0, &bind_group_layouts.0));
                    }
                }

                commands.entity(entity).insert(JointsOffset(
                    joint_buffers.buffers[joint_buffers.next_buffer]
                        .staging
                        .len() as u32,
                ));

                'joint_loop: for joint in animation_joints
                    .0
                    .iter(
                        &animated_model
                            .0
                            .animation_data
                            .joint_indices_to_node_indices,
                        &animated_model.0.animation_data.inverse_bind_transforms,
                        &animated_model.0.animation_data.depth_first_nodes,
                    )
                    .map(|joint| {
                        shared_structs::JointTransform::new(
                            joint.translation,
                            joint.scale,
                            joint.rotation,
                        )
                    })
                {
                    let next_buffer = joint_buffers.next_buffer;

                    if let Err(error) = joint_buffers.buffers[next_buffer].staging.try_push(joint) {
                        log::warn!("Got an error when pushing joints: {}", error);
                        break 'joint_loop;
                    }
                }
            }
            Err(error) => {
                log::warn!("Got an error when pushing joints: {}", error);
            }
        }
    })
}

pub(crate) fn push_entity_instances(
    camera: Res<Camera>,
    culling_params: Res<CullingParams>,
    surface_frame_view: Option<Res<SurfaceFrameView>>,
    mut instance_query: Query<(&InstanceOf, &Instance, Option<&JointsOffset>)>,
    mut model_query: Query<(&mut Instances, Option<&Model>, Option<&AnimatedModel>)>,
) {
    let view_matrix = camera.view_matrix();

    instance_query.for_each_mut(|(instance_of, instance, joints_offset)| {
        match model_query.get_mut(instance_of.0) {
            Ok((mut instances, model, animated_model)) => {
                if let Some(model) = model {
                    instances.reserve_space(&model.0.primitives);

                    for (primitive_id, primitive) in model.0.primitives.iter().enumerate() {
                        let primitive_transform = instance.0 * primitive.transform;

                        // calculate the size of the min z frustum rectangle or something (I have removed min_z from both sides of the equation).
                        // https://github.com/BabylonJS/Babylon.js/blob/d25bc29091d47f51bd2f0f98fb0f16d25517675f/packages/dev/core/src/Cameras/camera.ts#L149-L150
                        // todo: research more.
                        let screen_coverage = {
                            let distance_to_camera =
                                primitive_transform.translation.distance(camera.position);
                            let bounding_sphere_radius =
                                primitive.bounding_sphere.radius * primitive_transform.scale;
                            let visible_radius = bounding_sphere_radius / distance_to_camera;
                            let mesh_area = visible_radius * visible_radius * std::f32::consts::PI;
                            // There isn't a way to get the window dimensions in WebXR mode so we just use default values.
                            let (width, height) = surface_frame_view
                                .as_ref()
                                .map(|view| (view.width, view.height))
                                .unwrap_or((1024, 1024));
                            let aspect_ratio = width as f32 / height as f32;

                            let screen_area = {
                                let y = (59.0_f32.to_radians() / 2.0).tan();
                                let x = y * aspect_ratio;
                                x * y
                            };

                            mesh_area / screen_area
                        };

                        // Chose the lod that the screen coverage fits into.
                        let lod = match primitive.screen_coverages.binary_search_by(|value| {
                            screen_coverage
                                .partial_cmp(value)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }) {
                            Ok(exact) => exact,
                            Err(closest) => closest,
                        };

                        let mut passed_culling_check = match culling_params.bounding_sphere_params {
                            BoundingSphereParams::SingleView(params) => {
                                renderer_core::culling::test_bounding_sphere(
                                    primitive.bounding_sphere,
                                    primitive_transform,
                                    params,
                                )
                            }
                            BoundingSphereParams::Vr { left, right } => {
                                renderer_core::culling::test_bounding_sphere(
                                    primitive.bounding_sphere,
                                    primitive_transform,
                                    left,
                                ) || renderer_core::culling::test_bounding_sphere(
                                    primitive.bounding_sphere,
                                    primitive_transform,
                                    right,
                                )
                            }
                        };

                        if let Some(frustum) = culling_params.frustum {
                            passed_culling_check &=
                                renderer_core::culling::test_using_separating_axis_theorem(
                                    frustum,
                                    view_matrix,
                                    primitive_transform,
                                    &primitive.bounding_box,
                                );
                        }

                        if !passed_culling_check {
                            continue;
                        }

                        instances.insert(
                            primitive_id,
                            lod,
                            GpuInstance {
                                similarity: primitive_transform,
                                joints_offset: joints_offset.map(|offset| offset.0).unwrap_or(0),
                                material_index: primitive.lods[lod].material_index as u32,
                                is_lightmapped: primitive.lods[lod].is_lightmapped as u32,
                                _padding: Default::default(),
                            },
                        );
                    }
                } else if let Some(animated_model) = animated_model {
                    instances.reserve_space(&animated_model.0.primitives);

                    for (primitive_id, primitive) in animated_model.0.primitives.iter().enumerate()
                    {
                        let primitive_transform = instance.0 * primitive.transform;

                        // todo: culling for animated models.
                        instances.insert(
                            primitive_id,
                            0,
                            GpuInstance {
                                similarity: primitive_transform,
                                joints_offset: joints_offset.map(|offset| offset.0).unwrap_or(0),
                                material_index: primitive.lods[0].material_index as u32,
                                is_lightmapped: false as u32,
                                _padding: Default::default(),
                            },
                        );
                    }
                }
            }
            Err(error) => {
                log::warn!("Got an error when pushing an instance: {}", error);
            }
        }
    })
}

pub(crate) fn upload_instances(
    device: Res<Device>,
    queue: Res<Queue>,
    mut instance_buffer: ResMut<InstanceBuffer>,
    mut query: Query<(&Instances, &mut InstanceRanges)>,
) {
    let mut command_encoder = device
        .0
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("command encoder"),
        });

    query.for_each_mut(|(instances, mut instance_ranges)| {
        instance_ranges.clear();

        for primitives in instances.primitives.iter() {
            for (lod_index, lod) in primitives.lods.iter().enumerate() {
                instance_ranges.push(
                    lod_index,
                    instance_buffer.0.push(
                        &lod.instances,
                        &device.0,
                        &queue.0,
                        &mut command_encoder,
                    ),
                );
            }
        }
    });

    queue.0.submit(std::iter::once(command_encoder.finish()));
}

pub(crate) fn upload_lines(
    device: Res<Device>,
    queue: Res<Queue>,
    mut line_buffer: ResMut<LineBuffer>,
) {
    let mut command_encoder = device
        .0
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("command encoder"),
        });

    let LineBuffer { staging, buffer } = &mut *line_buffer;
    buffer.push(staging, &device.0, &queue.0, &mut command_encoder);

    queue.0.submit(std::iter::once(command_encoder.finish()));
}

pub(crate) fn upload_particles(
    device: Res<Device>,
    queue: Res<Queue>,
    camera: Res<Camera>,
    mut particle_buffer: ResMut<ParticleBuffer>,
) {
    let mut command_encoder = device
        .0
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("command encoder"),
        });

    let ParticleBuffer { staging, buffer } = &mut *particle_buffer;

    staging.sort_unstable_by_key(|p| {
        std::cmp::Reverse(ordered_float::OrderedFloat(
            p.position.distance_squared(camera.position),
        ))
    });

    buffer.push(staging, &device.0, &queue.0, &mut command_encoder);

    queue.0.submit(std::iter::once(command_encoder.finish()));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn allocate_bind_groups(
    device: Res<Device>,
    queue: Res<Queue>,
    bind_group_layouts: Res<BindGroupLayouts>,
    texture_settings: Res<TextureSettings>,
    mut commands: Commands,
) {
    use renderer_core::mutable_bind_group::Entry;

    let device = &device.0;
    let bind_group_layouts = &bind_group_layouts.0;

    let uniform_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniform buffer"),
        size: std::mem::size_of::<shared_structs::Uniforms>() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    }));

    let clamp_sampler = Arc::new(device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: texture_settings.0.anisotropy_clamp,
        ..Default::default()
    }));

    let create_texture_with_brightness = |label, brightness, dimension, view_dimension| {
        let desc = &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            sample_count: 1,
            mip_level_count: 1,
            dimension,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_formats: &[],
        };

        Entry::Texture(Arc::new(Texture::new_with_view_dimension(
            device.create_texture_with_data(&queue.0, desc, &[brightness; 4]),
            view_dimension,
        )))
    };

    let main_bind_group = Arc::new(MutableBindGroup::new(
        device,
        &bind_group_layouts.uniform,
        vec![
            Entry::Buffer(uniform_buffer.clone(), 0),
            Entry::Sampler(clamp_sampler),
            Entry::Texture(Arc::new(Texture::new_cubemap(device.create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("dummy ibl cubemap"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 6,
                    },
                    sample_count: 1,
                    mip_level_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_formats: &[],
                },
            )))),
            create_texture_with_brightness(
                "lightvol l0",
                255,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2Array,
            ),
            create_texture_with_brightness(
                "lightvol l1x",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2Array,
            ),
            create_texture_with_brightness(
                "lightvol l1y",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2Array,
            ),
            create_texture_with_brightness(
                "lightvol l1z",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2Array,
            ),
            create_texture_with_brightness(
                "lightmap l0",
                255,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "lightmap l1x",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "lightmap l1y",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "lightmap l1z",
                128,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "smoke a",
                255,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "smoke b",
                255,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
            create_texture_with_brightness(
                "smoke lut",
                0,
                wgpu::TextureDimension::D2,
                wgpu::TextureViewDimension::D2,
            ),
        ],
    ));

    commands.insert_resource(UniformBuffer(uniform_buffer));
    commands.insert_resource(MainBindGroup::new(main_bind_group));

    commands.insert_resource(IndexBuffer(Arc::new(renderer_core::IndexBuffer::new(
        1024, device,
    ))));
    commands.insert_resource(VertexBuffers(Arc::new(renderer_core::VertexBuffers::new(
        1024, device,
    ))));
    commands.insert_resource(AnimatedVertexBuffers(Arc::new(
        renderer_core::AnimatedVertexBuffers::new(1024, device),
    )));

    commands.insert_resource(InstanceBuffer(renderer_core::VecGpuBuffer::new(
        1,
        device,
        wgpu::BufferUsages::VERTEX,
        "instance buffer",
    )));

    commands.insert_resource(LineBuffer {
        buffer: renderer_core::VecGpuBuffer::new(
            1,
            device,
            wgpu::BufferUsages::VERTEX,
            "line buffer",
        ),
        staging: Vec::new(),
    });

    commands.insert_resource(ParticleBuffer {
        buffer: renderer_core::VecGpuBuffer::new(
            1,
            device,
            wgpu::BufferUsages::VERTEX,
            "particle buffer",
        ),
        staging: Vec::new(),
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_lightvol_textures<T: assets::HttpClient>(
    device: Res<Device>,
    queue: Res<Queue>,
    pipelines: Res<Pipelines>,
    bind_group_layouts: Res<BindGroupLayouts>,
    texture_settings: Res<TextureSettings>,
    mut new_lightvol_textures: ResMut<NewLightvolTextures>,
    main_bind_group: Res<MainBindGroup>,
    http_client: Res<HttpClient<T>>,
) {
    let new_lightvol_textures = match new_lightvol_textures.0.take() {
        Some(new_lightvol_textures) => new_lightvol_textures,
        None => return,
    };

    let lightvol_z_layers = main_bind_group.lightvol_z_layers.clone();
    let main_bind_group = main_bind_group.inner.clone();

    let device = &device.0;
    let queue = &queue.0;
    let pipelines = &pipelines.0;
    let bind_group_layouts = &bind_group_layouts.0;

    let features = device.features();

    let context = renderer_core::assets::textures::Context {
        device: device.clone(),
        queue: queue.clone(),
        http_client: http_client.0.clone(),
        bind_group_layouts: bind_group_layouts.clone(),
        pipelines: pipelines.clone(),
        settings: texture_settings.0.clone(),
    };

    spawn(async move {
        use renderer_core::assets::textures::load_ktx2_async;

        let lightvol_future = futures::future::join4(
            load_ktx2_async(&context, &new_lightvol_textures.sh0, false, |_| ()),
            load_ktx2_async(&context, &new_lightvol_textures.sh1_x, false, |_| ()),
            load_ktx2_async(&context, &new_lightvol_textures.sh1_y, false, |_| ()),
            load_ktx2_async(&context, &new_lightvol_textures.sh1_z, false, |_| ()),
        );

        let lightmap_future = futures::future::join4(
            load_ktx2_async(
                &context,
                new_lightvol_textures.lightmap_sh0.choose(features),
                false,
                |_| (),
            ),
            load_ktx2_async(
                &context,
                new_lightvol_textures.lightmap_sh1_x.choose(features),
                false,
                |_| (),
            ),
            load_ktx2_async(
                &context,
                new_lightvol_textures.lightmap_sh1_y.choose(features),
                false,
                |_| (),
            ),
            load_ktx2_async(
                &context,
                new_lightvol_textures.lightmap_sh1_z.choose(features),
                false,
                |_| (),
            ),
        );

        let smoke_a_url = url::Url::parse(
            "http://localhost:8000/assets/smoke/burst/TX_Pyro_AerialBurst_N.tga.ktx2",
        )
        .unwrap();
        let smoke_b_url = url::Url::parse(
            "http://localhost:8000/assets/smoke/burst/TX_Pyro_AerialBurst_P.tga.ktx2",
        )
        .unwrap();
        let smoke_emission_lut_url =
            url::Url::parse("http://localhost:8000/assets/smoke/lut.ktx2").unwrap();

        let smoke_future = futures::future::join3(
            load_ktx2_async(&context, &smoke_a_url, false, |_| ()),
            load_ktx2_async(&context, &smoke_b_url, false, |_| ()),
            load_ktx2_async(&context, &smoke_emission_lut_url, false, |_| ()),
        );

        let (
            (sh0, sh1_x, sh1_y, sh1_z),
            (lightmap_sh0, lightmap_sh1_x, lightmap_sh1_y, lightmap_sh1_z),
            (smoke_a, smoke_b, smoke_emission_lut),
        ) = futures::future::join3(lightvol_future, lightmap_future, smoke_future).await;

        let (sh0, sh1_x, sh1_y, sh1_z) = (sh0?, sh1_x?, sh1_y?, sh1_z?);
        let (lightmap_sh0, lightmap_sh1_x, lightmap_sh1_y, lightmap_sh1_z) = (
            lightmap_sh0?,
            lightmap_sh1_x?,
            lightmap_sh1_y?,
            lightmap_sh1_z?,
        );

        let (smoke_a, smoke_b, smoke_emission_lut) = (smoke_a?, smoke_b?, smoke_emission_lut?);

        use renderer_core::mutable_bind_group::Entry::Texture;

        lightvol_z_layers.store(sh0.texture.size().depth_or_array_layers, Ordering::Relaxed);
        main_bind_group.mutate(
            &context.device,
            &context.bind_group_layouts.uniform,
            |entries| {
                entries[3] = Texture(sh0);
                entries[4] = Texture(sh1_x);
                entries[5] = Texture(sh1_y);
                entries[6] = Texture(sh1_z);
                entries[7] = Texture(lightmap_sh0);
                entries[8] = Texture(lightmap_sh1_x);
                entries[9] = Texture(lightmap_sh1_y);
                entries[10] = Texture(lightmap_sh1_z);
                entries[11] = Texture(smoke_a);
                entries[12] = Texture(smoke_b);
                entries[13] = Texture(smoke_emission_lut);
            },
        );

        Ok(())
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_ibl_resources<T: assets::HttpClient>(
    device: Res<Device>,
    queue: Res<Queue>,
    pipelines: Res<Pipelines>,
    bind_group_layouts: Res<BindGroupLayouts>,
    texture_settings: Res<TextureSettings>,
    mut new_ibl_cubemap: ResMut<NewIblCubemap>,
    main_bind_group: Res<MainBindGroup>,
    http_client: Res<HttpClient<T>>,
) {
    let new_ibl_cubemap = match new_ibl_cubemap.0.take() {
        Some(new_ibl_cubemap) => new_ibl_cubemap,
        None => return,
    };

    let main_bind_group = main_bind_group.inner.clone();

    let device = &device.0;
    let queue = &queue.0;
    let pipelines = &pipelines.0;
    let bind_group_layouts = &bind_group_layouts.0;

    let textures_context = renderer_core::assets::textures::Context {
        device: device.clone(),
        queue: queue.clone(),
        http_client: http_client.0.clone(),
        bind_group_layouts: bind_group_layouts.clone(),
        pipelines: pipelines.clone(),
        settings: texture_settings.0.clone(),
    };

    spawn(async move {
        match renderer_core::assets::textures::load_ibl_cubemap(
            textures_context.clone(),
            &new_ibl_cubemap,
        )
        .await
        {
            Ok(texture) => {
                main_bind_group.mutate(
                    &textures_context.device,
                    &textures_context.bind_group_layouts.uniform,
                    |entries| {
                        entries[2] = renderer_core::mutable_bind_group::Entry::Texture(texture);
                    },
                );

                Ok(())
            }
            Err(error) => Err(anyhow::anyhow!(
                "Error file loading ibl cubemap {}: {}",
                new_ibl_cubemap,
                error
            )),
        }
    });
}

pub(crate) fn update_desktop_uniform_buffers(
    pipeline_options: Res<PipelineOptions>,
    queue: Res<Queue>,
    uniform_buffer: Res<UniformBuffer>,
    surface_frame_view: Res<SurfaceFrameView>,
    camera: Res<Camera>,
    probes_array: Res<ProbesArrayInfo>,
    main_bind_group: Res<MainBindGroup>,
    mut culling_params: ResMut<CullingParams>,
) {
    let queue = &queue.0;

    let perspective_matrix = Mat4::perspective_infinite_reverse_rh(
        59.0_f32.to_radians(),
        surface_frame_view.width as f32 / surface_frame_view.height as f32,
        0.001,
    );

    *culling_params =
        CullingParams {
            frustum: Some(CullingFrustum::new(
                59.0_f32.to_radians(),
                surface_frame_view.width as f32 / surface_frame_view.height as f32,
                0.001,
                1000.0,
            )),
            bounding_sphere_params: BoundingSphereParams::SingleView(
                BoundingSphereCullingParams::new(camera.view_matrix(), perspective_matrix, 0.001),
            ),
        };

    let projection_view = perspective_matrix * camera.view_matrix();

    let mut settings = Settings::REVERSE_Z;

    // Do srgb conversion in the shader if we're rendering to a non-srgb format.
    if !pipeline_options.0.framebuffer_format.is_srgb() {
        settings |= Settings::INLINE_SRGB;
    }

    if pipeline_options.0.inline_tonemapping {
        settings |= Settings::INLINE_TONEMAPPING;
    }

    let uniforms = renderer_core::shared_structs::Uniforms {
        left_projection_view: projection_view.into(),
        right_projection_view: projection_view.into(),
        left_projection_inverse: perspective_matrix.inverse().into(),
        right_projection_inverse: perspective_matrix.inverse().into(),
        left_view_inverse: camera.rotation.into(),
        right_view_inverse: camera.rotation.into(),
        left_view: camera.view_matrix().into(),
        right_view: camera.view_matrix().into(),
        left_view_inverse_matrix: camera.view_matrix().inverse().into(),
        right_view_inverse_matrix: camera.view_matrix().inverse().into(),
        left_projection: perspective_matrix.into(),
        right_projection: perspective_matrix.into(),
        left_eye_x: camera.position.x,
        left_eye_y: camera.position.y,
        left_eye_z: camera.position.z,
        right_eye_x: camera.position.x,
        right_eye_y: camera.position.y,
        right_eye_z: camera.position.z,
        probes_array_scale_x: probes_array.scale.x,
        probes_array_scale_y: probes_array.scale.y,
        probes_array_scale_z: probes_array.scale.z,
        probes_array_bottom_left_x: probes_array.bottom_left.x,
        probes_array_bottom_left_y: probes_array.bottom_left.y,
        probes_array_bottom_left_z: probes_array.bottom_left.z,
        settings,
        lightvol_z_layers: main_bind_group.lightvol_z_layers.load(Ordering::Relaxed),
        _padding: Default::default(),
    };

    queue.write_buffer(
        &uniform_buffer.0,
        0,
        renderer_core::bytemuck::bytes_of(&uniforms),
    );
}

#[cfg(feature = "webgl")]
#[derive(Default)]
struct ViewData {
    projection: Mat4,
    view: Mat4,
    instance: renderer_core::Instance,
}

#[cfg(feature = "webgl")]
pub(crate) fn update_webxr_uniform_buffers(
    mut camera: ResMut<Camera>,
    pose: bevy_ecs::prelude::NonSend<web_sys::XrViewerPose>,
    pipeline_options: Res<PipelineOptions>,
    queue: Res<Queue>,
    uniform_buffer: Res<UniformBuffer>,
    probes_array: Res<ProbesArrayInfo>,
    main_bind_group: Res<MainBindGroup>,
    mut culling_params: ResMut<CullingParams>,
) {
    let queue = &queue.0;

    let parse_matrix = |vec| Mat4::from_cols_array(&<[f32; 16]>::try_from(vec).unwrap());

    let views = pose.views();

    let mut views_iter = views.iter();

    let left_view: web_sys::XrView = views_iter.next().unwrap().into();

    let left_view_data = ViewData {
        projection: parse_matrix(left_view.projection_matrix()),
        view: parse_matrix(left_view.transform().matrix()).inverse(),
        instance: renderer_core::instance::instance_from_transform(left_view.transform(), 0.0),
    };

    let (right_view_data, is_vr) = if let Some(right_view) = views_iter.next() {
        let right_view: web_sys::XrView = right_view.into();

        (
            ViewData {
                projection: parse_matrix(right_view.projection_matrix()),
                view: parse_matrix(right_view.transform().matrix()).inverse(),
                instance: renderer_core::instance::instance_from_transform(
                    right_view.transform(),
                    0.0,
                ),
            },
            true,
        )
    } else {
        (Default::default(), false)
    };

    // Update the camera position for code that uses that (like lod selection).
    camera.position =
        (left_view_data.instance.translation + right_view_data.instance.translation) / 2.0;

    let mut settings = Settings::INLINE_SRGB;

    if pipeline_options.0.flip_viewport {
        settings |= Settings::FLIP_VIEWPORT;
    }

    if pipeline_options.0.inline_tonemapping {
        settings |= Settings::INLINE_TONEMAPPING;
    }

    let uniforms = renderer_core::shared_structs::Uniforms {
        left_projection_view: (left_view_data.projection * left_view_data.view).into(),
        right_projection_view: (right_view_data.projection * right_view_data.view).into(),
        left_projection_inverse: left_view_data.projection.inverse().into(),
        right_projection_inverse: right_view_data.projection.inverse().into(),
        left_projection: left_view_data.projection.into(),
        right_projection: right_view_data.projection.into(),
        left_view: left_view_data.view.into(),
        right_view: right_view_data.view.into(),
        left_view_inverse_matrix: left_view_data.view.inverse().into(),
        right_view_inverse_matrix: right_view_data.view.inverse().into(),
        left_view_inverse: left_view_data.instance.rotation.into(),
        right_view_inverse: right_view_data.instance.rotation.into(),
        left_eye_x: left_view_data.instance.translation.x,
        left_eye_y: left_view_data.instance.translation.y,
        left_eye_z: left_view_data.instance.translation.z,
        right_eye_x: right_view_data.instance.translation.x,
        right_eye_y: right_view_data.instance.translation.y,
        right_eye_z: right_view_data.instance.translation.z,
        probes_array_scale_x: probes_array.scale.x,
        probes_array_scale_y: probes_array.scale.y,
        probes_array_scale_z: probes_array.scale.z,
        probes_array_bottom_left_x: probes_array.bottom_left.x,
        probes_array_bottom_left_y: probes_array.bottom_left.y,
        probes_array_bottom_left_z: probes_array.bottom_left.z,
        settings,
        lightvol_z_layers: main_bind_group.lightvol_z_layers.load(Ordering::Relaxed),
        _padding: Default::default(),
    };

    queue.write_buffer(
        &uniform_buffer.0,
        0,
        renderer_core::bytemuck::bytes_of(&uniforms),
    );

    *culling_params = CullingParams {
        frustum: None,
        bounding_sphere_params: if is_vr {
            BoundingSphereParams::Vr {
                left: BoundingSphereCullingParams::new(
                    left_view_data.view,
                    left_view_data.projection,
                    0.001,
                ),
                right: BoundingSphereCullingParams::new(
                    right_view_data.view,
                    right_view_data.projection,
                    0.001,
                ),
            }
        } else {
            BoundingSphereParams::SingleView(BoundingSphereCullingParams::new(
                left_view_data.view,
                left_view_data.projection,
                0.001,
            ))
        },
    };
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn start_loading_models<T: assets::HttpClient>(
    static_models: Query<(Entity, &ModelUrl), Added<ModelUrl>>,
    animated_models: Query<(Entity, &AnimatedModelUrl), Added<AnimatedModelUrl>>,
    device: Res<Device>,
    queue: Res<Queue>,
    pipelines: Res<Pipelines>,
    bind_group_layouts: Res<BindGroupLayouts>,
    (index_buffer, vertex_buffers, animated_vertex_buffers): (
        Res<IndexBuffer>,
        Res<VertexBuffers>,
        Res<AnimatedVertexBuffers>,
    ),
    texture_settings: Res<TextureSettings>,
    http_client: Res<HttpClient<T>>,
    mut commands: Commands,
) {
    let device = &device.0;
    let queue = &queue.0;

    static_models.for_each(|(entity, url)| {
        let url = url.0.clone();
        let vertex_buffers = vertex_buffers.0.clone();
        let animated_vertex_buffers = animated_vertex_buffers.0.clone();
        let index_buffer = index_buffer.0.clone();
        let texture_settings = texture_settings.0.clone();

        let model_setter = Arc::new(ArcSwapOption::empty());

        commands
            .entity(entity)
            .insert(PendingModel(model_setter.clone()));

        spawn({
            let device = device.clone();
            let queue = queue.clone();
            let bind_group_layouts = bind_group_layouts.0.clone();
            let pipelines = pipelines.0.clone();

            let context = renderer_core::assets::models::Context {
                device,
                queue,
                bind_group_layouts,
                http_client: http_client.0.clone(),
                index_buffer,
                vertex_buffers,
                animated_vertex_buffers,
                pipelines,
                texture_settings,
            };

            async move {
                let result = renderer_core::assets::models::Model::load(&context, &url).await;

                match result {
                    Ok(model) => {
                        model_setter.store(Some(Arc::new(model)));

                        Ok(())
                    }
                    Err(error) => Err(anyhow::anyhow!(
                        "Got an error while trying to load a model from '{}': {}",
                        url,
                        error
                    )),
                }
            }
        });
    });

    animated_models.for_each(|(entity, url)| {
        let url = url.0.clone();
        let vertex_buffers = vertex_buffers.0.clone();
        let animated_vertex_buffers = animated_vertex_buffers.0.clone();
        let index_buffer = index_buffer.0.clone();
        let texture_settings = texture_settings.0.clone();

        let model_setter = Arc::new(ArcSwapOption::empty());

        commands
            .entity(entity)
            .insert(PendingAnimatedModel(model_setter.clone()));

        spawn({
            let device = device.clone();
            let queue = queue.clone();
            let bind_group_layouts = bind_group_layouts.0.clone();
            let pipelines = pipelines.0.clone();

            let context = renderer_core::assets::models::Context {
                device,
                queue,
                bind_group_layouts,
                http_client: http_client.0.clone(),
                index_buffer,
                vertex_buffers,
                animated_vertex_buffers,
                pipelines,
                texture_settings,
            };

            async move {
                let result =
                    renderer_core::assets::models::AnimatedModel::load(&context, &url).await;

                match result {
                    Ok(model) => {
                        model_setter.store(Some(Arc::new(model)));
                        Ok(())
                    }
                    Err(error) => Err(anyhow::anyhow!(
                        "Got an error while trying to load a model from '{}': {}",
                        url,
                        error
                    )),
                }
            }
        });
    });
}

pub(crate) fn finish_loading_models(
    static_models: Query<(Entity, &PendingModel)>,
    animated_models: Query<(Entity, &PendingAnimatedModel)>,
    device: Res<Device>,
    bind_group_layouts: Res<BindGroupLayouts>,
    mut commands: Commands,
) {
    static_models.for_each(|(entity, pending_model)| {
        if let Some(loaded_model) = pending_model.0.swap(None) {
            commands.entity(entity).insert(Model(loaded_model));
        }
    });

    animated_models.for_each(|(entity, pending_model)| {
        if let Some(loaded_model) = pending_model.0.swap(None) {
            commands
                .entity(entity)
                .insert(AnimatedModel(loaded_model))
                .insert(JointBuffers::new(&device.0, &bind_group_layouts.0));
        }
    })
}

pub(crate) fn add_joints_to_instances(
    animated_models: Query<&AnimatedModel>,
    instances: Query<(Entity, &InstanceOf), Without<AnimationJoints>>,
    mut commands: Commands,
) {
    instances.for_each(|(entity, instance_of)| {
        if let Ok(animated_model) = animated_models.get(instance_of.0) {
            commands.entity(entity).insert(AnimationJoints(
                animated_model.0.animation_data.animation_joints.clone(),
            ));
        }
    })
}
