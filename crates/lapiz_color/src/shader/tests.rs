use std::{
    array::from_fn,
    sync::{Arc, mpsc},
};

use lapiz_runtime::renderer::global_render_context;
use moxcms::{
    CicpColorPrimaries, CicpProfile, DataColorSpace, Lab, LutDataType, LutMultidimensionalType,
    LutStore, LutType, LutWarehouse, Matrix3d, MatrixCoefficients, RenderingIntent, ToneReprCurve,
    TransferCharacteristics, Vector3d, Xyzd,
};
#[allow(
    clippy::wildcard_imports,
    reason = "wesl quote macros resolve syntax types from this prelude"
)]
use wesl::syntax::*;
use wesl_quote::quote_module;
use wgpu::{
    BindGroupDescriptor, BindGroupLayoutDescriptor, BufferDescriptor, BufferUsages,
    COPY_BYTES_PER_ROW_ALIGNMENT, Color, ColorTargetState, ColorWrites, CommandEncoderDescriptor,
    Extent3d, FragmentState, LoadOp, MapMode, Operations, PipelineLayoutDescriptor, PollType,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, VertexState,
    util::{BufferInitDescriptor, DeviceExt as _},
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

fn trc_options() -> TransformOptions {
    TransformOptions {
        allow_use_cicp_transfer: false,
        ..Default::default()
    }
}

fn reference(src: &ColorProfile, dst: &ColorProfile, input: [f32; 3]) -> [f32; 3] {
    let src_trcs = [&src.red_trc, &src.green_trc, &src.blue_trc];
    let dst_trcs = [&dst.red_trc, &dst.green_trc, &dst.blue_trc];
    let linear = from_fn::<_, 3, _>(|i| {
        src_trcs[i]
            .as_ref()
            .unwrap()
            .make_linear_evaluator()
            .unwrap()
            .evaluate_value(input[i])
    });
    let matrix = src.transform_matrix(dst).v;
    from_fn(|i| {
        let value = (0..3)
            .map(|j| matrix[i][j] * f64::from(linear[j]))
            .sum::<f64>() as f32;
        dst_trcs[i]
            .as_ref()
            .unwrap()
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
        let shader =
            IccTransformShader::new("convert", src, dst, trc_options(), &context.device).unwrap();
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
    let shader =
        IccTransformShader::new("convert", &src, &dst, trc_options(), &context.device).unwrap();
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
        let shader =
            IccTransformShader::new("convert", &src, &dst, trc_options(), &context.device).unwrap();
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
                    trc_options(),
                    &context.device,
                )
                .is_err()
            );
        }
        let directions = [(&profile, &linear), (&linear, &profile)];
        for &(src, dst) in &directions[..if invertible { 2 } else { 1 }] {
            let shader =
                IccTransformShader::new("convert", src, dst, trc_options(), &context.device)
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
    let shader =
        IccTransformShader::new("convert", &linear, &profile, trc_options(), &context.device)
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
        trc_options(),
        &context.device,
    )
    .unwrap();
    let unmanaged = pipeline::build("convert", None, None, trc_options(), &context.device).unwrap();
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
        IccTransformShader::new("convert", src, dst, trc_options(), &context.device)
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
        trc_options(),
        &context.device,
    )
    .unwrap();
    let inputs = [[0.01, 0.25, 0.5], [0.8, 0.6, 0.1]];
    for (actual, expected) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
        assert_close(actual, expected, 0.00001);
    }
}

fn identity_xyz_profile() -> ColorProfile {
    let mut profile = ColorProfile::new_srgb();
    profile.cicp = None;
    profile.red_colorant = Xyzd::new(1.0, 0.0, 0.0);
    profile.green_colorant = Xyzd::new(0.0, 1.0, 0.0);
    profile.blue_colorant = Xyzd::new(0.0, 0.0, 1.0);
    profile.red_trc = Some(ToneReprCurve::Lut(Vec::new()));
    profile.green_trc = profile.red_trc.clone();
    profile.blue_trc = profile.red_trc.clone();
    profile
}

#[test]
fn cicp_transfer_switch_matches_evaluators_in_both_directions() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let inputs = [[0.0; 3], [0.01, 0.2, 0.5], [0.75, 0.9, 1.0]];
    for transfer in [
        TransferCharacteristics::Linear,
        TransferCharacteristics::Srgb,
        TransferCharacteristics::Bt709,
        TransferCharacteristics::Bt470M,
        TransferCharacteristics::Bt470Bg,
        TransferCharacteristics::Bt601,
        TransferCharacteristics::Smpte240,
        TransferCharacteristics::Log100,
        TransferCharacteristics::Log100sqrt10,
        TransferCharacteristics::Iec61966,
        TransferCharacteristics::Bt1361,
        TransferCharacteristics::Bt202010bit,
        TransferCharacteristics::Bt202012bit,
        TransferCharacteristics::Smpte428,
        TransferCharacteristics::Smpte2084,
        TransferCharacteristics::Hlg,
        TransferCharacteristics::Reserved,
        TransferCharacteristics::Unspecified,
    ] {
        let mut profile = linear.clone();
        profile.cicp = Some(CicpProfile {
            color_primaries: CicpColorPrimaries::Bt709,
            transfer_characteristics: transfer,
            matrix_coefficients: MatrixCoefficients::Identity,
            full_range: true,
        });
        for enabled in [false, true] {
            for inverse in [false, true] {
                let (src, dst) = if inverse {
                    (&linear, &profile)
                } else {
                    (&profile, &linear)
                };
                let shader = IccTransformShader::new(
                    "convert",
                    src,
                    dst,
                    TransformOptions {
                        allow_use_cicp_transfer: enabled,
                        ..Default::default()
                    },
                    &context.device,
                )
                .unwrap();
                let evaluator = if inverse {
                    ToneReprCurve::make_cicp_gamma_evaluator(transfer)
                } else {
                    ToneReprCurve::make_cicp_linear_evaluator(transfer)
                };
                for (actual, input) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
                    let expected = input.map(|v| match (&evaluator, enabled) {
                        (Ok(evaluator), true) => evaluator.evaluate_value(v),
                        _ => v,
                    });
                    for (actual, expected) in actual.into_iter().zip(expected) {
                        assert!(
                            (actual - expected).abs() <= 0.00002,
                            "transfer {transfer:?}, enabled {enabled}, inverse {inverse}: actual {actual}, expected {expected}"
                        );
                    }
                }
            }
        }

        let shader = IccTransformShader::new(
            "convert",
            &profile,
            &profile,
            TransformOptions::default(),
            &context.device,
        )
        .unwrap();
        let dark_inputs = [[0.0, 0.000001, 0.0001], [0.001, 0.01, 1.0]];
        for (actual, input) in run_shader(&shader, &dark_inputs)
            .into_iter()
            .zip(dark_inputs)
        {
            let expected = input.map(|v| {
                if matches!(
                    transfer,
                    TransferCharacteristics::Reserved | TransferCharacteristics::Unspecified
                ) {
                    v
                } else {
                    transfer.gamma(transfer.linearize(f64::from(v))) as f32
                }
            });
            assert_close(actual, expected, 0.00002);
        }
    }
}

fn legacy_lut(value: u16) -> LutWarehouse {
    LutWarehouse::Lut(LutDataType {
        num_input_channels: 3,
        num_output_channels: 3,
        num_clut_grid_points: 2,
        matrix: Matrix3d::IDENTITY,
        num_input_table_entries: 2,
        num_output_table_entries: 2,
        input_table: LutStore::Store16(vec![0, 65535, 0, 65535, 0, 65535]),
        clut_table: LutStore::Store16(vec![value; 24]),
        output_table: LutStore::Store16(vec![0, 65535, 0, 65535, 0, 65535]),
        lut_type: LutType::Lut16,
    })
}

#[test]
fn rendering_intent_selects_source_and_destination_luts() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let mut profile = linear.clone();
    profile.lut_a_to_b_perceptual = Some(legacy_lut(8192));
    profile.lut_a_to_b_colorimetric = Some(legacy_lut(16384));
    profile.lut_a_to_b_saturation = Some(legacy_lut(24576));
    profile.lut_b_to_a_perceptual = Some(legacy_lut(8192));
    profile.lut_b_to_a_colorimetric = Some(legacy_lut(16384));
    profile.lut_b_to_a_saturation = Some(legacy_lut(24576));
    let inputs = [[0.1, 0.2, 0.3], [0.9, 0.7, 0.5]];
    for (intent, value) in [
        (RenderingIntent::Perceptual, 8192.0),
        (RenderingIntent::RelativeColorimetric, 16384.0),
        (RenderingIntent::AbsoluteColorimetric, 16384.0),
        (RenderingIntent::Saturation, 24576.0),
    ] {
        for inverse in [false, true] {
            let (src, dst) = if inverse {
                (&linear, &profile)
            } else {
                (&profile, &linear)
            };
            let shader = IccTransformShader::new(
                "convert",
                src,
                dst,
                TransformOptions {
                    rendering_intent: intent,
                    ..Default::default()
                },
                &context.device,
            )
            .unwrap();
            assert_eq!(shader.bindings(5).len(), 7);
            let expected = value / if inverse { 65535.0 } else { 32768.0 };
            for actual in run_shader(&shader, &inputs) {
                assert_close(actual, [expected; 3], 0.00001);
            }
        }
    }
}

fn multidimensional_lut() -> LutMultidimensionalType {
    let mut clut = Vec::new();
    for r in 0..2 {
        for g in 0..3 {
            for b in 0..4 {
                clut.extend([r as u16 * 65535, g as u16 * 30000, b as u16 * 20000]);
            }
        }
    }
    let mut grid_points = [0; 16];
    grid_points[..3].copy_from_slice(&[2, 3, 4]);
    LutMultidimensionalType {
        num_input_channels: 3,
        num_output_channels: 3,
        grid_points,
        clut: Some(LutStore::Store16(clut)),
        a_curves: vec![ToneReprCurve::Parametric(vec![2.0]); 3],
        b_curves: vec![ToneReprCurve::Parametric(vec![2.0]); 3],
        m_curves: vec![ToneReprCurve::Parametric(vec![2.0]); 3],
        matrix: Matrix3d {
            v: [[0.8, 0.1, 0.0], [0.0, 0.7, 0.1], [0.1, 0.0, 0.6]],
        },
        bias: Vector3d {
            v: [0.01, 0.02, 0.03],
        },
    }
}

#[test]
fn multidimensional_stages_follow_a_to_b_and_b_to_a_order() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let lut = multidimensional_lut();
    let mut profile = linear.clone();
    profile.lut_a_to_b_perceptual = Some(LutWarehouse::Multidimensional(lut.clone()));
    profile.lut_b_to_a_perceptual = Some(LutWarehouse::Multidimensional(lut.clone()));
    let inputs = [[0.1, 0.3, 0.5], [0.8, 0.7, 0.6], [1.0; 3]];
    let curves = |v: [f32; 3]| v.map(|v| v.clamp(0.0, 1.0).powi(2));
    let clut = |v: [f32; 3]| {
        [
            v[0].clamp(0.0, 1.0),
            v[1].clamp(0.0, 1.0) * 60000.0 / 65535.0,
            v[2].clamp(0.0, 1.0) * 60000.0 / 65535.0,
        ]
    };
    let matrix = |v: [f32; 3]| {
        from_fn(|i| {
            (lut.bias.v[i]
                + (0..3)
                    .map(|j| lut.matrix.v[i][j] * f64::from(v[j]))
                    .sum::<f64>()) as f32
        })
    };
    for inverse in [false, true] {
        let (src, dst) = if inverse {
            (&linear, &profile)
        } else {
            (&profile, &linear)
        };
        let shader =
            IccTransformShader::new("convert", src, dst, trc_options(), &context.device).unwrap();
        for (actual, input) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
            let expected = if inverse {
                curves(clut(curves(matrix(curves(
                    input.map(|v| v * 32768.0 / 65535.0),
                )))))
            } else {
                curves(matrix(curves(clut(curves(input))))).map(|v| v * 65535.0 / 32768.0)
            };
            assert_close(actual, expected, 0.00001);
        }
    }
}

#[test]
fn lab_pcs_supports_legacy_and_multidimensional_encoding() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let inputs = [[0.01, 0.02, 0.03], [0.2, 0.3, 0.4]];
    for legacy in [false, true] {
        let mut profile = linear.clone();
        profile.pcs = DataColorSpace::Lab;
        let lut = if legacy {
            legacy_lut(32768)
        } else {
            let mut lut = multidimensional_lut();
            lut.a_curves.clear();
            lut.m_curves.clear();
            lut.clut = None;
            lut.b_curves = vec![ToneReprCurve::Lut(Vec::new()); 3];
            LutWarehouse::Multidimensional(lut)
        };
        profile.lut_a_to_b_perceptual = Some(lut.clone());
        profile.lut_b_to_a_perceptual = Some(lut);
        for inverse in [false, true] {
            let (src, dst) = if inverse {
                (&linear, &profile)
            } else {
                (&profile, &linear)
            };
            let shader =
                IccTransformShader::new("convert", src, dst, trc_options(), &context.device)
                    .unwrap();
            for (actual, input) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
                let expected = if inverse {
                    if legacy {
                        [32768.0 / 65535.0; 3]
                    } else {
                        let lab = Lab::from_pcs_xyz(moxcms::Xyz::new(
                            input[0] * 32768.0 / 65535.0,
                            input[1] * 32768.0 / 65535.0,
                            input[2] * 32768.0 / 65535.0,
                        ));
                        [lab.l, lab.a, lab.b]
                    }
                } else {
                    let c = if legacy {
                        [32768.0 / 65280.0; 3]
                    } else {
                        input
                    };
                    let xyz = Lab::new(c[0], c[1], c[2]).to_pcs_xyz();
                    [xyz.x, xyz.y, xyz.z].map(|v| v * 65535.0 / 32768.0)
                };
                assert_close(actual, expected, 0.00001);
            }
        }
    }
}

#[test]
fn absolute_intent_scales_media_whites_for_matrix_and_lut_profiles() {
    let context = global_render_context();
    let mut src = identity_xyz_profile();
    let mut dst = identity_xyz_profile();
    src.media_white_point = Some(Xyzd::new(0.8, 1.0, 0.6));
    dst.media_white_point = Some(Xyzd::new(1.0, 1.0, 0.8));
    let inputs = [[0.1, 0.2, 0.3], [0.5; 3]];
    for lut in [false, true] {
        if lut {
            src.lut_a_to_b_colorimetric = Some(legacy_lut(8192));
        }
        let shader = IccTransformShader::new(
            "convert",
            &src,
            &dst,
            TransformOptions {
                rendering_intent: RenderingIntent::AbsoluteColorimetric,
                ..trc_options()
            },
            &context.device,
        )
        .unwrap();
        for (actual, input) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
            let xyz = if lut { [0.25; 3] } else { input };
            assert_close(actual, [xyz[0] * 0.8, xyz[1], xyz[2] * 0.75], 0.00001);
        }
    }
}

#[test]
fn legacy_tables_and_xyz_matrix_run_in_order() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let mut profile = linear.clone();
    let LutWarehouse::Lut(mut lut) = legacy_lut(0) else {
        unreachable!()
    };
    lut.matrix = Matrix3d {
        v: [[0.0, 0.5, 0.0], [0.0, 0.0, 0.75], [0.25, 0.0, 0.0]],
    };
    lut.input_table = LutStore::Store16(vec![0, 32768, 0, 32768, 0, 32768]);
    lut.output_table = LutStore::Store16(vec![0, 49152, 0, 49152, 0, 49152]);
    let mut clut = Vec::new();
    for r in [0, 65535] {
        for g in [0, 65535] {
            for b in [0, 65535] {
                clut.extend([r, g, b]);
            }
        }
    }
    lut.clut_table = LutStore::Store16(clut);
    profile.lut_a_to_b_perceptual = Some(LutWarehouse::Lut(lut.clone()));
    profile.lut_b_to_a_perceptual = Some(LutWarehouse::Lut(lut));
    let inputs = [[0.1, 0.3, 0.7], [-0.2, 0.25, 1.4]];
    for inverse in [false, true] {
        let (src, dst) = if inverse {
            (&linear, &profile)
        } else {
            (&profile, &linear)
        };
        let shader =
            IccTransformShader::new("convert", src, dst, trc_options(), &context.device).unwrap();
        for (actual, input) in run_shader(&shader, &inputs).into_iter().zip(inputs) {
            let v = if inverse {
                let v = input.map(|v| v * 32768.0 / 65535.0);
                [v[1] * 0.5, v[2] * 0.75, v[0] * 0.25]
            } else {
                input
            };
            let expected = v.map(|v| {
                v.clamp(0.0, 1.0) * 32768.0 / 65535.0 * 49152.0 / 65535.0
                    * if inverse { 1.0 } else { 65535.0 / 32768.0 }
            });
            assert_close(actual, expected, 0.00001);
        }
    }
}

#[test]
fn eight_bit_lab_lut_and_combined_lut_profiles_preserve_pcs() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let mut source = linear.clone();
    source.pcs = DataColorSpace::Lab;
    let LutWarehouse::Lut(mut lut) = legacy_lut(0) else {
        unreachable!()
    };
    lut.lut_type = LutType::Lut8;
    lut.input_table = LutStore::Store8(vec![0, 255, 0, 255, 0, 255]);
    lut.output_table = lut.input_table.clone();
    lut.clut_table = LutStore::Store8([128, 128, 128].repeat(8));
    source.lut_a_to_b_perceptual = Some(LutWarehouse::Lut(lut));
    let mut destination = linear.clone();
    let mut xyz_lut = multidimensional_lut();
    xyz_lut.a_curves.clear();
    xyz_lut.clut = None;
    xyz_lut.m_curves.clear();
    xyz_lut.b_curves = vec![ToneReprCurve::Lut(Vec::new()); 3];
    destination.lut_b_to_a_perceptual = Some(LutWarehouse::Multidimensional(xyz_lut));
    let shader = IccTransformShader::new(
        "convert",
        &source,
        &destination,
        trc_options(),
        &context.device,
    )
    .unwrap();
    assert_eq!(shader.bindings(5).len(), 7);
    let xyz = Lab::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0).to_pcs_xyz();
    for actual in run_shader(&shader, &[[0.1, 0.2, 0.3], [0.5; 3]]) {
        assert_close(actual, [xyz.x, xyz.y, xyz.z], 0.00001);
    }
}

#[test]
fn input_and_output_transforms_use_separate_curve_bindings() {
    let context = global_render_context();
    let sampled = sampled_profile();
    let xyz = identity_xyz_profile();
    let input = IccInputTransformShader::new("convert", &sampled, &context.device).unwrap();
    let output = IccOutputTransformShader::new("convert", &sampled, &context.device).unwrap();
    let inputs = [[0.01, 0.2, 0.5], [0.8, 0.7, 0.6]];

    for (shader, src, dst) in [
        (&input.transform, &sampled, &xyz),
        (&output.transform, &xyz, &sampled),
    ] {
        let (_, layout) = shader.compile(1, 5, ShaderStages::FRAGMENT);
        let bindings = shader.bindings(5);
        assert_eq!(bindings.len(), 3);
        assert_eq!(layout.len(), 3);
        for (i, (binding, entry)) in bindings.iter().zip(&layout).enumerate() {
            assert_eq!(binding.binding, 5 + i as u32);
            assert_eq!(entry.binding, binding.binding);
        }
        for (actual, input) in run_shader(shader, &inputs).into_iter().zip(inputs) {
            assert_close(actual, reference(src, dst, input), 0.0001);
        }
    }
}

#[test]
fn lut_validation_reports_missing_intent_and_malformed_tables() {
    let context = global_render_context();
    let linear = identity_xyz_profile();
    let mut profile = linear.clone();
    profile.lut_a_to_b_perceptual = Some(legacy_lut(16384));
    assert!(
        IccTransformShader::new(
            "convert",
            &profile,
            &linear,
            TransformOptions {
                rendering_intent: RenderingIntent::Saturation,
                ..Default::default()
            },
            &context.device
        )
        .is_err()
    );
    for lut in [
        {
            let mut lut = multidimensional_lut();
            lut.grid_points[0] = 0;
            lut
        },
        {
            let mut lut = multidimensional_lut();
            lut.b_curves.pop();
            lut
        },
        {
            let mut lut = multidimensional_lut();
            lut.clut = Some(LutStore::Store16(vec![0]));
            lut
        },
        {
            let mut lut = multidimensional_lut();
            lut.num_output_channels = 4;
            lut
        },
    ] {
        profile.lut_a_to_b_perceptual = Some(LutWarehouse::Multidimensional(lut));
        assert!(
            IccTransformShader::new(
                "convert",
                &profile,
                &linear,
                TransformOptions::default(),
                &context.device
            )
            .is_err()
        );
    }
}
