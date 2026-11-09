use std::{
    array::from_fn,
    sync::{Arc, mpsc},
};

use lapiz_runtime::renderer::global_render_context;
use wesl::syntax::{BuiltinValue, TranslationUnit, UnaryExpression, UnaryOperator};
use wesl_quote::quote_module;
use wgpu::{
    BindGroupDescriptor, BindGroupLayoutDescriptor, BufferDescriptor, COPY_BYTES_PER_ROW_ALIGNMENT,
    Color, ColorTargetState, ColorWrites, CommandEncoderDescriptor, Extent3d, FragmentState,
    LoadOp, MapMode, Operations, PipelineLayoutDescriptor, PollType, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, StoreOp,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, VertexState, util::DeviceExt as _,
};

use super::*;

fn run_shader(shader: &IccTransformShader, input: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let context = global_render_context();
    let device = &context.device;
    device.on_uncaptured_error(Arc::new(|error| panic!("{error}")));
    let queue = &context.queue;
    let (mut source, trc_layout) = shader.compile(1, 5, ShaderStages::FRAGMENT);
    source.push_str(
        &quote_module! {
            @group(0) @binding(0) var<storage, read> inputs: array<vec4f>;
            @vertex
            fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
                let positions = array<vec2f, 3>(vec2f(-1.0f, -1.0f), vec2f(3.0f, -1.0f), vec2f(-1.0f, 3.0f));
                return vec4f(positions[index], 0.0f, 1.0f);
            }
            @fragment
            fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
                return vec4f(convert(inputs[u32(position.x)].xyz), 1.0f);
            }
        }
        .to_string(),
    );
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("ICC test shader"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let entries = [BindGroupLayoutEntry {
        binding: 0,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
    });
    let trc_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("ICC TRC layout"),
        entries: &trc_layout,
    });
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout), Some(&trc_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("ICC test pipeline"),
        layout: Some(&pipeline_layout),
        vertex: VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(ColorTargetState {
                format: TextureFormat::Rgba32Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let input_bytes = input
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 1.0])
        .flat_map(f32::to_ne_bytes)
        .collect::<Vec<_>>();
    let input_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: None,
        contents: &input_bytes,
        usage: BufferUsages::STORAGE,
    });
    let size = Extent3d {
        width: input.len() as u32,
        height: 1,
        depth_or_array_layers: 1,
    };
    let output = device.create_texture(&TextureDescriptor {
        label: Some("ICC test output"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba32Float,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let bytes_per_row = (input_bytes.len() as u32).div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT)
        * COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(bytes_per_row),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let resources = [BindGroupEntry {
        binding: 0,
        resource: input_buffer.as_entire_binding(),
    }];
    let bindings = device.create_bind_group(&BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &resources,
    });
    let trc_bindings = device.create_bind_group(&BindGroupDescriptor {
        label: Some("ICC TRC bindings"),
        layout: &trc_layout,
        entries: &shader.bindings(5),
    });
    let view = output.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::TRANSPARENT),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.set_bind_group(1, &trc_bindings, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(1),
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = mpsc::channel();
    readback.map_async(MapMode::Read, .., move |result| tx.send(result).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = readback.get_mapped_range(..);
    mapped[..input_bytes.len()]
        .chunks_exact(16)
        .map(|pixel| from_fn(|i| f32::from_ne_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())))
        .collect()
}

fn assert_close(actual: [f32; 3], expected: [f32; 3], tolerance: f32) {
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= tolerance,
            "channel {channel}: actual {actual}, expected {expected}, tolerance {tolerance}"
        );
    }
}

fn reference(src: &ColorProfile, dst: &ColorProfile, input: [f32; 3]) -> [f32; 3] {
    let linear = from_fn::<_, 3, _>(|i| {
        profile_trcs(src).unwrap()[i]
            .make_linear_evaluator()
            .unwrap()
            .evaluate_value(input[i])
    });
    let matrix = src.transform_matrix(dst).v;
    from_fn(|i| {
        let value = (0..3)
            .map(|j| matrix[i][j] * f64::from(linear[j]))
            .sum::<f64>() as f32;
        profile_trcs(dst).unwrap()[i]
            .make_gamma_evaluator()
            .unwrap()
            .evaluate_value(value)
    })
}

fn sampled_profile() -> ColorProfile {
    let mut profile = ColorProfile::new_srgb();
    profile.red_trc = Some(ToneReprCurve::Lut(vec![0, 3000, 15000, 37000, 65535]));
    profile.green_trc = Some(ToneReprCurve::Lut(vec![0, 8000, 24000, 44000, 65535]));
    profile.blue_trc = Some(ToneReprCurve::Lut(vec![0, 16000, 65535]));
    profile
}

#[test]
fn sampled_curves_match_moxcms_in_both_directions() {
    let context = global_render_context();
    let sampled = sampled_profile();
    let srgb = ColorProfile::new_srgb();
    let inputs = (0..=256)
        .map(|i| {
            let x = i as f32 / 256.0;
            [x, 1.0 - x, (x * 7.0).fract()]
        })
        .chain([[0.0; 3], [1.0; 3]])
        .collect::<Vec<_>>();
    for (src, dst, expected_tables) in [
        (&sampled, &srgb, 3),
        (&srgb, &sampled, 3),
        (&sampled, &sampled, 6),
    ] {
        let shader = IccTransformShader::new(
            "convert",
            src,
            dst,
            TransformOptions::default(),
            &context.device,
        )
        .unwrap();
        let (_, layout) = shader.compile(1, 5, ShaderStages::FRAGMENT);
        assert_eq!(shader.bindings(5).len(), expected_tables);
        assert_eq!(layout.len(), expected_tables);
        for (i, (binding, entry)) in shader.bindings(5).iter().zip(&layout).enumerate() {
            assert_eq!(binding.binding, 5 + i as u32);
            assert_eq!(entry.binding, 5 + i as u32);
            assert_eq!(entry.visibility, ShaderStages::FRAGMENT);
        }
        for (input, actual) in inputs.iter().zip(run_shader(&shader, &inputs)) {
            assert_close(actual, reference(src, dst, *input), 0.0001);
        }
    }
}

#[test]
fn sampled_curves_clamp_and_preserve_table_endpoints() {
    let context = global_render_context();
    let mut src = sampled_profile();
    src.red_trc = Some(ToneReprCurve::Lut(vec![4000, 16000, 60000]));
    let mut dst = src.clone();
    dst.red_trc = Some(ToneReprCurve::Lut(Vec::new()));
    dst.green_trc = dst.red_trc.clone();
    dst.blue_trc = dst.red_trc.clone();
    let shader = IccTransformShader::new(
        "convert",
        &src,
        &dst,
        TransformOptions::default(),
        &context.device,
    )
    .unwrap();
    let inputs = [[-0.5; 3], [0.0; 3], [0.25; 3], [0.5; 3], [1.0; 3], [1.5; 3]];
    for (input, actual) in inputs.into_iter().zip(run_shader(&shader, &inputs)) {
        assert_close(actual, reference(&src, &dst, input), 0.00001);
    }
}

#[test]
fn analytic_and_mixed_curves_match_moxcms() {
    let context = global_render_context();
    let mut mixed = sampled_profile();
    mixed.green_trc = Some(ToneReprCurve::Lut(Vec::new()));
    mixed.blue_trc = Some(ToneReprCurve::Lut(vec![512]));
    let inputs = [
        [0.01, 0.03, 0.05],
        [0.04045, 0.25, 0.5],
        [0.18, 0.6, 0.8],
        [0.9, 0.8, 0.95],
    ];
    for (src, dst) in [
        (ColorProfile::new_srgb(), ColorProfile::new_display_p3()),
        (ColorProfile::new_adobe_rgb(), ColorProfile::new_srgb()),
        (mixed.clone(), ColorProfile::new_srgb()),
        (ColorProfile::new_srgb(), mixed),
    ] {
        let shader = IccTransformShader::new(
            "convert",
            &src,
            &dst,
            TransformOptions::default(),
            &context.device,
        )
        .unwrap();
        for (input, actual) in inputs.into_iter().zip(run_shader(&shader, &inputs)) {
            assert_close(actual, reference(&src, &dst, input), 0.0001);
        }
    }
}

#[test]
fn parametric_curve_types_match_moxcms() {
    let context = global_render_context();
    let mut linear = ColorProfile::new_srgb();
    linear.red_trc = Some(ToneReprCurve::Lut(Vec::new()));
    linear.green_trc = linear.red_trc.clone();
    linear.blue_trc = linear.red_trc.clone();
    let inputs = [[0.01; 3], [0.04; 3], [0.05; 3], [0.25; 3], [0.8; 3]];
    for (parameters, invertible) in [
        (vec![2.2], true),
        (vec![2.2, 0.9, 0.1], true),
        (vec![2.2, 0.9, 0.1, 0.0], true),
        // moxcms requires finite inverse coefficients for the constant lower branch.
        (vec![2.2, 0.9, 0.1, 0.01], false),
        (
            vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
            true,
        ),
        (vec![2.0, 0.9, 0.1, 0.2, 0.04, 0.01, 0.02], true),
    ] {
        let mut profile = linear.clone();
        profile.red_trc = Some(ToneReprCurve::Parametric(parameters));
        profile.green_trc = profile.red_trc.clone();
        profile.blue_trc = profile.red_trc.clone();
        if !invertible {
            assert!(
                profile
                    .red_trc
                    .as_ref()
                    .unwrap()
                    .make_gamma_evaluator()
                    .is_err()
            );
            assert!(
                IccTransformShader::new(
                    "convert",
                    &linear,
                    &profile,
                    TransformOptions::default(),
                    &context.device,
                )
                .is_err()
            );
        }
        let directions = [(&profile, &linear), (&linear, &profile)];
        for &(src, dst) in &directions[..if invertible { 2 } else { 1 }] {
            let shader = IccTransformShader::new(
                "convert",
                src,
                dst,
                TransformOptions::default(),
                &context.device,
            )
            .unwrap();
            for (input, actual) in inputs.into_iter().zip(run_shader(&shader, &inputs)) {
                assert_close(actual, reference(src, dst, input), 0.0001);
            }
        }
    }
}

#[test]
fn inverse_sampled_curves_handle_plateaus_and_two_entry_tables() {
    let context = global_render_context();
    let mut linear = ColorProfile::new_srgb();
    linear.red_trc = Some(ToneReprCurve::Lut(Vec::new()));
    linear.green_trc = linear.red_trc.clone();
    linear.blue_trc = linear.red_trc.clone();
    let mut profile = linear.clone();
    profile.red_trc = Some(ToneReprCurve::Lut(vec![0, 0, 16000, 16000, 65535, 65535]));
    profile.green_trc = Some(ToneReprCurve::Lut(vec![4000, 60000]));
    profile.blue_trc = Some(ToneReprCurve::Lut((0..=u16::MAX).collect()));
    let shader = IccTransformShader::new(
        "convert",
        &linear,
        &profile,
        TransformOptions::default(),
        &context.device,
    )
    .unwrap();
    let inputs = [
        [-0.1; 3], [0.0; 3], [0.1; 3], [0.25; 3], [0.5; 3], [1.0; 3], [1.1; 3],
    ];
    for (input, actual) in inputs.into_iter().zip(run_shader(&shader, &inputs)) {
        assert_close(actual, reference(&linear, &profile, input), 0.0001);
    }
}

#[test]
fn analytic_and_unmanaged_transforms_have_empty_bindings() {
    let context = global_render_context();
    let profile = ColorProfile::new_srgb();
    let managed = IccTransformShader::new(
        "convert",
        &profile,
        &profile,
        TransformOptions::default(),
        &context.device,
    )
    .unwrap();
    let unmanaged = IccTransformShader::unmanaged("convert");
    for shader in [&managed, &unmanaged] {
        let (_, layout) = shader.compile(1, 8, ShaderStages::FRAGMENT);
        assert!(layout.is_empty());
        assert!(shader.bindings(8).is_empty());
    }
    let inputs = [[-1.0, 0.25, 2.0]];
    assert_eq!(run_shader(&unmanaged, &inputs), inputs);
}

#[test]
fn invalid_profiles_and_curves_return_errors() {
    let context = global_render_context();
    let generate = |src: &ColorProfile, dst: &ColorProfile| {
        IccTransformShader::new(
            "convert",
            src,
            dst,
            TransformOptions::default(),
            &context.device,
        )
    };
    let srgb = ColorProfile::new_srgb();
    let mut invalid = srgb.clone();
    invalid.color_space = DataColorSpace::Gray;
    assert!(generate(&invalid, &srgb).is_err());
    invalid = srgb.clone();
    invalid.red_trc = None;
    assert!(generate(&srgb, &invalid).is_err());
    for curve in [
        ToneReprCurve::Parametric(Vec::new()),
        ToneReprCurve::Parametric(vec![1.0, 2.0]),
        ToneReprCurve::Parametric(vec![f32::NAN]),
        ToneReprCurve::Lut(vec![0]),
    ] {
        invalid = srgb.clone();
        invalid.red_trc = Some(curve);
        assert!(generate(&invalid, &srgb).is_err());
        assert!(generate(&srgb, &invalid).is_err());
    }
}

#[test]
fn managed_identity_transform_preserves_colors() {
    let context = global_render_context();
    let profile = ColorProfile::new_srgb();
    let shader = IccTransformShader::new(
        "convert",
        &profile,
        &profile,
        TransformOptions::default(),
        &context.device,
    )
    .unwrap();
    let inputs = [[0.01, 0.25, 0.5], [0.8, 0.6, 0.1]];
    for (actual, expected) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
        assert_close(actual, expected, 0.00001);
    }
}
