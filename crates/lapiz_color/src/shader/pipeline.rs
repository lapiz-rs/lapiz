use std::array::from_fn;

use anyhow::{Result, ensure};
use moxcms::{
    Chromaticity, CmsError, ColorProfile, DataColorSpace, LutStore, LutType, LutWarehouse,
    Matrix3d, ParametricCurve, RenderingIntent, ToneReprCurve, TransferCharacteristics,
    TransformOptions,
};
#[allow(
    clippy::wildcard_imports,
    reason = "wesl quote macros resolve syntax types from this prelude"
)]
use wesl::syntax::*;
use wesl_quote::{quote_declaration, quote_statement};
use wgpu::{
    Buffer, BufferUsages, Device,
    util::{BufferInitDescriptor, DeviceExt as _},
};

use crate::shader::{IccTransformShader, TABLES_GROUP_CONST, TABLES_START_BINDING_CONST};

pub(super) fn build(
    ident: &str,
    src: Option<&ColorProfile>,
    dst: Option<&ColorProfile>,
    options: TransformOptions,
    device: &Device,
) -> Result<IccTransformShader> {
    let mut builder = Pipeline {
        prefix: ident,
        shader: String::new(),
        device,
        buffers: Vec::new(),
        next: 0,
    };
    let mut stages = Vec::new();
    if let Some(src) = src {
        stages.push(builder.profile_to_xyz_function(src, options, false)?);
    }
    if let (Some(src), Some(dst), RenderingIntent::AbsoluteColorimetric) =
        (src, dst, options.rendering_intent)
    {
        let src_white = src.media_white_point.unwrap_or(src.white_point);
        let dst_white = dst.media_white_point.unwrap_or(dst.white_point);
        let source = [src_white.x, src_white.y, src_white.z];
        let destination = [dst_white.x, dst_white.y, dst_white.z];
        ensure!(
            source
                .iter()
                .chain(&destination)
                .all(|v| v.is_finite() && *v > 0.0),
            "Positive finite media white points required"
        );
        let [x, y, z] = from_fn(|i| (source[i] / destination[i]) as f32);
        ensure!(
            [x, y, z].iter().all(|v| v.is_finite()),
            "Finite media white scaling required"
        );
        let scale = builder.next_function_name();
        // Full chromatic adaptation preserves the PCS matrix and scales media whites.
        builder.shader.push_str(
            &quote_declaration! {
                fn #scale(c: vec3f) -> vec3f { return c * vec3f(#x, #y, #z); }
            }
            .to_string(),
        );
        stages.push(scale);
    }
    if let Some(dst) = dst {
        stages.push(builder.profile_to_xyz_function(dst, options, true)?);
    }
    builder.compose_function(&Ident::new(ident.to_owned()), &stages);

    Ok(IccTransformShader {
        shader: builder.shader,
        table_buffers: builder.buffers,
    })
}

/// Internal builder that accumulates generated WGSL stage functions and GPU lookup table buffers for an [`IccTransformShader`].
struct Pipeline<'a> {
    prefix: &'a str,
    shader: String,
    device: &'a Device,
    buffers: Vec<Buffer>,
    next: u32,
}

impl Pipeline<'_> {
    fn next_function_name(&mut self) -> Ident {
        let name = Ident::new(format!("_{}_stage_{}", self.prefix, self.next));
        self.next += 1;
        name
    }

    fn compose_function(&mut self, ident: &Ident, stages: &[Ident]) {
        let GlobalDeclaration::Function(mut function) = (quote_declaration! {
            fn #ident(c: vec3f) -> vec3f { var color = c; }
        }) else {
            unreachable!()
        };
        for stage in stages {
            function.body.statements.push(
                (quote_statement! {
                    color = #stage(color);
                })
                .into(),
            );
        }
        function
            .body
            .statements
            .push((quote_statement! { return color; }).into());
        self.shader
            .push_str(&GlobalDeclaration::Function(function).to_string());
    }

    fn table_binding(&mut self, ident: &Ident, samples: &[f32]) -> Result<()> {
        let bytes = size_of_val(samples) as u64;
        ensure!(
            bytes <= self.device.limits().max_storage_buffer_binding_size,
            "ICC table exceeds storage buffer binding limit"
        );
        ensure!(
            self.buffers.len() < self.device.limits().max_storage_buffers_per_shader_stage as usize,
            "ICC tables exceed storage buffer count limit"
        );

        let group = Ident::new(TABLES_GROUP_CONST.to_owned());
        let start_binding = Ident::new(TABLES_START_BINDING_CONST.to_owned());
        let binding = self.buffers.len() as u32;
        let len = samples.len() as u32;

        self.shader.push_str(&quote_declaration! {
            @group(#group) @binding(#start_binding + #binding) var<storage, read> #ident: array<f32, #len>;
        }.to_string());

        self.buffers
            .push(self.device.create_buffer_init(&BufferInitDescriptor {
                label: Some(&ident.to_string()),
                contents: bytemuck::cast_slice(samples),
                usage: BufferUsages::STORAGE,
            }));

        Ok(())
    }

    fn map_with_sampled_function(&mut self, samples: &[f32]) -> Result<Ident> {
        ensure!(
            samples.len() >= 2 && samples.iter().all(|v| v.is_finite()),
            "Finite curve samples required"
        );
        let ident = self.next_function_name();
        let table = Ident::new(format!("{ident}_table"));
        let len = samples.len();
        let last = len as u32 - 1;
        let scale = last as f32;

        self.table_binding(&table, samples)?;
        self.shader.push_str(
            &quote_declaration! {
                fn #ident(x: f32) -> f32 {
                    let position = clamp(x, 0.0f, 1.0f) * #scale;
                    let lower = u32(position);
                    let upper = min(lower + 1u, #last);
                    return mix(#table[lower], #table[upper], position - f32(lower));
                }
            }
            .to_string(),
        );
        Ok(ident)
    }

    fn map_color_with_matrix_function(&mut self, matrix: &Matrix3d) -> Result<Ident> {
        let ident = self.next_function_name();
        let m = matrix.v.map(|row| row.map(|value| value as f32));
        let [a, b, c] = m[0];
        let [d, e, f] = m[1];
        let [g, h, i] = m[2];
        self.shader.push_str(
            &quote_declaration! {
                fn #ident(x: vec3f) -> vec3f {
                    // WGSL matrices use columns, moxcms matrices use rows.
                    return mat3x3f(#a, #d, #g, #b, #e, #h, #c, #f, #i) * x;
                }
            }
            .to_string(),
        );
        Ok(ident)
    }

    fn map_with_trc_function(&mut self, curve: &ToneReprCurve, inverse: bool) -> Result<Ident> {
        let ident = self.next_function_name();
        match curve {
            ToneReprCurve::Lut(lut) if lut.is_empty() => {
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            return x;
                        }
                    }
                    .to_string(),
                );

                Ok(ident)
            }
            ToneReprCurve::Lut(lut) if lut.len() == 1 => {
                let gamma = f32::from(lut[0]) / 256.0;
                ensure!(gamma > 0.0, "Positive gamma required");
                let gamma = if inverse { gamma.recip() } else { gamma };
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            return pow(clamp(x, 0.0f, 1.0f), #gamma);
                        }
                    }
                    .to_string(),
                );
                Ok(ident)
            }
            ToneReprCurve::Lut(lut) => {
                let samples = if inverse {
                    const INVERSE_SAMPLES: u32 = 16384;
                    let evaluator = curve.make_gamma_evaluator()?;
                    (0..INVERSE_SAMPLES)
                        .map(|i| evaluator.evaluate_value(i as f32 / (INVERSE_SAMPLES - 1) as f32))
                        .collect::<Vec<_>>()
                } else {
                    lut.iter()
                        .map(|&value| f32::from(value) / 65535.0)
                        .collect()
                };

                self.map_with_sampled_function(&samples)
            }
            ToneReprCurve::Parametric(parameters) => {
                ensure!(
                    matches!(parameters.len(), 1 | 3 | 4 | 5 | 7)
                        && parameters.iter().all(|value| value.is_finite()),
                    "Valid finite parametric curve parameters required"
                );
                let curve =
                    ParametricCurve::new(parameters).ok_or(CmsError::BuildTransferFunction)?;
                let ParametricCurve {
                    g,
                    a,
                    b,
                    c,
                    d,
                    e,
                    f,
                } = if inverse {
                    curve.invert().ok_or(CmsError::BuildTransferFunction)?
                } else {
                    curve
                };

                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            if x < #d {
                                return #c * x + #f;
                            } else {
                                return pow(#a * x + #b, #g) + #e;
                            }
                        }
                    }
                    .to_string(),
                );
                Ok(ident)
            }
        }
    }

    fn map_rgb_with_function_function(&mut self, mapper: &[Ident; 3]) -> Ident {
        let ident = self.next_function_name();
        let [r, g, b] = mapper;
        self.shader.push_str(
            &quote_declaration! {
                fn #ident(c: vec3f) -> vec3f { return vec3f(#r(c.x), #g(c.y), #b(c.z)); }
            }
            .to_string(),
        );
        ident
    }

    fn map_rgb_with_trc_function(&mut self, curves: &[ToneReprCurve; 3]) -> Result<Ident> {
        let r = self.map_with_trc_function(&curves[0], false)?;
        let g = self.map_with_trc_function(&curves[1], false)?;
        let b = self.map_with_trc_function(&curves[2], false)?;
        let channels = self.map_rgb_with_function_function(&[r, g, b]);
        let ident = self.next_function_name();
        self.shader.push_str(
            &quote_declaration! {
                fn #ident(c: vec3f) -> vec3f {
                    return #channels(clamp(c, vec3f(0.0f), vec3f(1.0f)));
                }
            }
            .to_string(),
        );
        Ok(ident)
    }

    fn profile_to_xyz_function(
        &mut self,
        profile: &ColorProfile,
        options: TransformOptions,
        inverse: bool,
    ) -> Result<Ident> {
        ensure!(
            profile.color_space == DataColorSpace::Rgb,
            "RGB color space required"
        );
        let mut stages = Vec::new();

        let has_lut = if inverse {
            profile.lut_b_to_a_perceptual.is_some()
                || profile.lut_b_to_a_colorimetric.is_some()
                || profile.lut_b_to_a_saturation.is_some()
        } else {
            profile.lut_a_to_b_perceptual.is_some()
                || profile.lut_a_to_b_colorimetric.is_some()
                || profile.lut_a_to_b_saturation.is_some()
        };

        if has_lut {
            let lut = match (inverse, options.rendering_intent) {
                (false, RenderingIntent::Perceptual) => &profile.lut_a_to_b_perceptual,
                (false, RenderingIntent::Saturation) => &profile.lut_a_to_b_saturation,
                (
                    false,
                    RenderingIntent::RelativeColorimetric | RenderingIntent::AbsoluteColorimetric,
                ) => &profile.lut_a_to_b_colorimetric,
                (true, RenderingIntent::Perceptual) => &profile.lut_b_to_a_perceptual,
                (true, RenderingIntent::Saturation) => &profile.lut_b_to_a_saturation,
                (
                    true,
                    RenderingIntent::RelativeColorimetric | RenderingIntent::AbsoluteColorimetric,
                ) => &profile.lut_b_to_a_colorimetric,
            }
            .as_ref()
            .ok_or(CmsError::UnsupportedLutRenderingIntent(
                options.rendering_intent,
            ))?;
            let pcs = self.pcs_to_xyz_function(profile.pcs, lut, inverse)?;
            let lut_stages = self.lut_to_stages(lut, inverse, profile.pcs)?;
            if inverse {
                stages.push(pcs);
                stages.extend(lut_stages);
            } else {
                stages.extend(lut_stages);
                stages.push(pcs);
            }
        } else {
            ensure!(
                profile.pcs == DataColorSpace::Xyz,
                "XYZ PCS required for matrix shaper"
            );

            let cicp = profile
                .cicp
                .filter(|_| options.allow_use_cicp_transfer)
                .map(|cicp| cicp.transfer_characteristics)
                .filter(|transfer| {
                    !matches!(
                        transfer,
                        TransferCharacteristics::Reserved | TransferCharacteristics::Unspecified
                    )
                });

            let curves = if let Some(transfer) = cicp {
                let curve = self.map_with_cicp_curve_function(transfer, inverse)?;
                self.map_rgb_with_function_function(&[curve.clone(), curve.clone(), curve])
            } else {
                ensure!(
                    profile.color_space == DataColorSpace::Rgb,
                    "RGB color space required"
                );
                ensure!(profile.is_matrix_shaper(), "Matrix shaper profile required");

                let r = self.map_with_trc_function(
                    profile
                        .red_trc
                        .as_ref()
                        .ok_or(CmsError::BuildTransferFunction)?,
                    inverse,
                )?;
                let g = self.map_with_trc_function(
                    profile
                        .green_trc
                        .as_ref()
                        .ok_or(CmsError::BuildTransferFunction)?,
                    inverse,
                )?;
                let b = self.map_with_trc_function(
                    profile
                        .blue_trc
                        .as_ref()
                        .ok_or(CmsError::BuildTransferFunction)?,
                    inverse,
                )?;
                self.map_rgb_with_function_function(&[r, g, b])
            };

            let matrix = if inverse {
                profile.rgb_to_xyz_matrix().inverse()
            } else {
                profile.rgb_to_xyz_matrix()
            };
            let transform = self.map_color_with_matrix_function(&matrix)?;
            if inverse {
                stages.extend([transform, curves]);
            } else {
                stages.extend([curves, transform]);
            }
        }
        let ident = self.next_function_name();
        self.compose_function(&ident, &stages);
        Ok(ident)
    }

    fn map_with_cicp_curve_function(
        &mut self,
        transfer: TransferCharacteristics,
        inverse: bool,
    ) -> Result<Ident> {
        let parameters = match transfer {
            TransferCharacteristics::Srgb => Some(vec![
                2.4,
                1.0 / 1.055_010_7,
                0.055_010_72 / 1.055_010_7,
                1.0 / 12.92,
                12.92 * 0.003_041_282_5,
            ]),
            TransferCharacteristics::Bt709
            | TransferCharacteristics::Bt601
            | TransferCharacteristics::Bt202010bit
            | TransferCharacteristics::Bt202012bit
            | TransferCharacteristics::Iec61966
            | TransferCharacteristics::Bt1361 => Some(vec![
                1.0 / 0.45,
                1.0 / 1.099_296_8,
                0.099_296_83 / 1.099_296_8,
                1.0 / 4.5,
                4.5 * 0.018_053_97,
            ]),
            TransferCharacteristics::Smpte240 => Some(vec![
                1.0 / 0.45,
                1.0 / 1.111_572_2,
                0.111_572_2 / 1.111_572_2,
                0.25,
                4.0 * 0.022_821_585,
            ]),
            TransferCharacteristics::Bt470M => Some(vec![2.2]),
            TransferCharacteristics::Bt470Bg => Some(vec![2.8]),
            _ => None,
        };
        if let Some(parameters) = parameters {
            let curve =
                self.map_with_trc_function(&ToneReprCurve::Parametric(parameters), inverse)?;
            let ident = self.next_function_name();
            self.shader.push_str(
                &quote_declaration! {
                    fn #ident(x: f32) -> f32 { return #curve(clamp(x, 0.0f, 1.0f)); }
                }
                .to_string(),
            );
            return Ok(ident);
        }

        if transfer == TransferCharacteristics::Linear {
            let ident = self.next_function_name();
            self.shader.push_str(
                &quote_declaration! {
                    fn #ident(x: f32) -> f32 { return clamp(x, 0.0f, 1.0f); }
                }
                .to_string(),
            );
            return Ok(ident);
        }

        if transfer == TransferCharacteristics::Smpte2084 {
            let ident = self.next_function_name();
            if inverse {
                self.shader.push_str(&quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        if x <= 0.0f { return 0.0f; }
                        let p = pow(min(x, 1.0f), 0.1593017578125f);
                        return pow(1.0f + (0.1640625f * p - 0.1640625f) / (1.0f + 18.6875f * p), 78.84375f);
                    }
                }.to_string());
            } else {
                self.shader.push_str(&quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        if x <= 0.0f { return 0.0f; }
                        let p = pow(x, 1.0f / 78.84375f);
                        return pow(max(p - 0.8359375f, 0.0f) / (18.8515625f - 18.6875f * p), 1.0f / 0.1593017578125f);
                    }
                }.to_string());
            }
            return Ok(ident);
        }

        if transfer == TransferCharacteristics::Hlg {
            let ident = self.next_function_name();
            if inverse {
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            let v = pow(clamp(x, 0.0f, 1.0f), 1.0f / 1.2f);
                            if v <= 1.0f / 12.0f { return sqrt(3.0f * v); }
                            return 0.17883277f * log(12.0f * v - 0.28466892f) + 0.55991073f;
                        }
                    }
                    .to_string(),
                );
            } else {
                self.shader.push_str(&quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        if x <= 0.0f { return 0.0f; }
                        if x <= 0.5f { return pow(x * x / 3.0f, 1.2f); }
                        return pow((exp((x - 0.55991073f) / 0.17883277f) + 0.28466892f) / 12.0f, 1.2f);
                    }
                }.to_string());
            }
            return Ok(ident);
        }

        if transfer == TransferCharacteristics::Smpte428 {
            let ident = self.next_function_name();
            let scale = 0.916_555_3_f32;
            if inverse {
                // The decoded SMPTE 428 endpoint exceeds one.
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            return pow(#scale * max(x, 0.0f), 1.0f / 2.6f);
                        }
                    }
                    .to_string(),
                );
            } else {
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            return pow(clamp(x, 0.0f, 1.0f), 2.6f) / #scale;
                        }
                    }
                    .to_string(),
                );
            }
            return Ok(ident);
        }

        if matches!(
            transfer,
            TransferCharacteristics::Log100 | TransferCharacteristics::Log100sqrt10
        ) {
            let ident = self.next_function_name();
            let (power, cutoff) = if transfer == TransferCharacteristics::Log100 {
                (2.0f32, 0.01f32)
            } else {
                (2.5f32, 0.003_162_277_6_f32)
            };
            // Analytic evaluation preserves the logarithmic curve's discontinuity at zero.
            if inverse {
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            if x <= #cutoff { return 0.0f; }
                            return 1.0f + log(min(x, 1.0f)) / (log(10.0f) * #power);
                        }
                    }
                    .to_string(),
                );
            } else {
                self.shader.push_str(
                    &quote_declaration! {
                        fn #ident(x: f32) -> f32 {
                            if x <= 0.0f { return #cutoff / 2.0f; }
                            return pow(10.0f, #power * (min(x, 1.0f) - 1.0f));
                        }
                    }
                    .to_string(),
                );
            }
            return Ok(ident);
        }

        Err(CmsError::BuildTransferFunction.into())
    }

    fn map_rgb_with_table_function(&mut self, table: &LutStore, entries: u16) -> Result<Ident> {
        let samples = samples(table);
        let entries = entries as usize;
        ensure!(
            entries >= 2 && samples.len() == entries * 3,
            "Three complete LUT curves required"
        );
        let r = self.map_with_sampled_function(&samples[..entries])?;
        let g = self.map_with_sampled_function(&samples[entries..entries * 2])?;
        let b = self.map_with_sampled_function(&samples[entries * 2..])?;
        Ok(self.map_rgb_with_function_function(&[r, g, b]))
    }

    fn lut_to_stages(
        &mut self,
        lut: &LutWarehouse,
        inverse: bool,
        pcs: DataColorSpace,
    ) -> Result<Vec<Ident>> {
        let mut stages = Vec::new();
        match lut {
            LutWarehouse::Lut(lut) => {
                ensure!(
                    lut.num_input_channels == 3 && lut.num_output_channels == 3,
                    "Three-channel ICC LUT required"
                );
                ensure!(
                    matches!(lut.lut_type, LutType::Lut8 | LutType::Lut16),
                    "lut8Type or lut16Type required"
                );
                ensure!(
                    lut.matrix
                        .v
                        .iter()
                        .flatten()
                        .all(|v| (*v as f32).is_finite()),
                    "Finite LUT matrix required"
                );
                ensure!(
                    pcs != DataColorSpace::Xyz || lut.lut_type == LutType::Lut16,
                    "XYZ PCS requires lut16Type"
                );
                // The legacy LUT matrix applies only to XYZ input.
                if inverse && pcs == DataColorSpace::Xyz {
                    stages.push(self.map_color_with_matrix_function(&lut.matrix)?);
                }
                stages.push(
                    self.map_rgb_with_table_function(
                        &lut.input_table,
                        lut.num_input_table_entries,
                    )?,
                );
                stages.push(self.map_color_with_clut_function(
                    &lut.clut_table,
                    [lut.num_clut_grid_points; 3],
                )?);
                stages.push(self.map_rgb_with_table_function(
                    &lut.output_table,
                    lut.num_output_table_entries,
                )?);
            }
            LutWarehouse::Multidimensional(lut) => {
                ensure!(
                    lut.num_input_channels == 3 && lut.num_output_channels == 3,
                    "Three-channel ICC LUT required"
                );
                ensure!(lut.b_curves.len() == 3, "Three B curves required");
                ensure!(
                    lut.a_curves.is_empty() == lut.clut.is_none(),
                    "A curves and CLUT must occur together"
                );
                let mut forward = Vec::new();
                if !lut.a_curves.is_empty() {
                    let curves = self
                        .map_rgb_with_trc_function(TryFrom::try_from(lut.a_curves.as_slice())?)?;
                    let clut = self.map_color_with_clut_function(
                        lut.clut.as_ref().unwrap(),
                        [lut.grid_points[0], lut.grid_points[1], lut.grid_points[2]],
                    )?;
                    if inverse {
                        forward.extend([clut, curves]);
                    } else {
                        forward.extend([curves, clut]);
                    }
                }
                let mut middle = Vec::new();
                if !lut.m_curves.is_empty() {
                    let curves = self
                        .map_rgb_with_trc_function(TryFrom::try_from(lut.m_curves.as_slice())?)?;
                    ensure!(
                        lut.matrix
                            .v
                            .iter()
                            .flatten()
                            .all(|v| (*v as f32).is_finite()),
                        "Finite LUT matrix required"
                    );
                    let matrix = self.map_color_with_matrix_function(&lut.matrix)?;
                    let bias = self.next_function_name();
                    let [x, y, z] = lut.bias.v.map(|v| v as f32);
                    ensure!(
                        [x, y, z].iter().all(|v| v.is_finite()),
                        "Finite LUT matrix bias required"
                    );
                    self.shader.push_str(
                        &quote_declaration! {
                            fn #bias(c: vec3f) -> vec3f { return #matrix(c) + vec3f(#x, #y, #z); }
                        }
                        .to_string(),
                    );
                    if inverse {
                        middle.extend([bias, curves]);
                    } else {
                        middle.extend([curves, bias]);
                    }
                }
                let b =
                    self.map_rgb_with_trc_function(TryFrom::try_from(lut.b_curves.as_slice())?)?;
                if inverse {
                    stages.push(b);
                    stages.extend(middle);
                    stages.extend(forward);
                } else {
                    stages.extend(forward);
                    stages.extend(middle);
                    stages.push(b);
                }
            }
        }
        Ok(stages)
    }

    fn map_color_with_clut_function(&mut self, lut: &LutStore, grid: [u8; 3]) -> Result<Ident> {
        ensure!(
            grid.iter().all(|&n| n >= 2),
            "CLUT grid dimensions must be at least two"
        );
        let samples = samples(lut);
        let [nx, ny, nz] = grid.map(u32::from);
        ensure!(
            samples.len() == (nx * ny * nz * 3) as usize,
            "Complete three-channel CLUT required"
        );
        let table = Ident::new(format!("{}_clut", self.next_function_name()));
        self.table_binding(&table, &samples)?;
        let fetch = self.next_function_name();
        self.shader.push_str(
            &quote_declaration! {
                fn #fetch(p: vec3u) -> vec3f {
                    let i = ((p.x * #ny + p.y) * #nz + p.z) * 3u;
                    return vec3f(#table[i], #table[i + 1u], #table[i + 2u]);
                }
            }
            .to_string(),
        );
        let ident = self.next_function_name();
        let [x, y, z] = grid.map(|v| f32::from(v - 1));
        let [ix, iy, iz] = grid.map(|v| u32::from(v - 1));
        self.shader.push_str(&quote_declaration! {
            fn #ident(c: vec3f) -> vec3f {
                let p = clamp(c, vec3f(0.0f), vec3f(1.0f)) * vec3f(#x, #y, #z);
                let lo = vec3u(p);
                let hi = min(lo + vec3u(1u), vec3u(#ix, #iy, #iz));
                let t = p - vec3f(lo);
                let a = mix(#fetch(lo), #fetch(vec3u(hi.x, lo.y, lo.z)), t.x);
                let b = mix(#fetch(vec3u(lo.x, hi.y, lo.z)), #fetch(vec3u(hi.x, hi.y, lo.z)), t.x);
                let d = mix(#fetch(vec3u(lo.x, lo.y, hi.z)), #fetch(vec3u(hi.x, lo.y, hi.z)), t.x);
                let e = mix(#fetch(vec3u(lo.x, hi.y, hi.z)), #fetch(hi), t.x);
                return mix(mix(a, b, t.y), mix(d, e, t.y), t.z);
            }
        }.to_string());
        Ok(ident)
    }

    fn pcs_to_xyz_function(
        &mut self,
        pcs: DataColorSpace,
        lut: &LutWarehouse,
        inverse: bool,
    ) -> Result<Ident> {
        ensure!(
            matches!(pcs, DataColorSpace::Xyz | DataColorSpace::Lab),
            "XYZ or Lab PCS required"
        );
        let ident = self.next_function_name();
        if pcs == DataColorSpace::Xyz {
            let scale = if inverse {
                32768.0f32 / 65535.0
            } else {
                65535.0f32 / 32768.0
            };
            self.shader.push_str(
                &quote_declaration! {
                    fn #ident(c: vec3f) -> vec3f { return c * #scale; }
                }
                .to_string(),
            );
            return Ok(ident);
        }

        // lut16Type uses the legacy 0xff00 encoding for Lab endpoints.
        let legacy = matches!(lut, LutWarehouse::Lut(lut) if lut.lut_type == LutType::Lut16);
        let scale = if legacy { 65535.0f32 / 65280.0 } else { 1.0 };
        let white = Chromaticity::D50.to_xyz();
        let (wx, wy, wz) = (white.x, white.y, white.z);
        let f = self.next_function_name();
        if inverse {
            self.shader.push_str(
                &quote_declaration! {
                    fn #f(t: f32) -> f32 {
                        if t > 216.0f / 24389.0f { return pow(t, 1.0f / 3.0f); }
                        return (24389.0f / 27.0f * t + 16.0f) / 116.0f;
                    }
                }
                .to_string(),
            );
            self.shader.push_str(
                &quote_declaration! {
                    fn #ident(c: vec3f) -> vec3f {
                        let v = c / vec3f(#wx, #wy, #wz);
                        let x = #f(v.x);
                        let y = #f(v.y);
                        let z = #f(v.z);
                        return vec3f((116.0f * y - 16.0f) / 100.0f,
                            (500.0f * (x - y) + 128.0f) / 255.0f,
                            (200.0f * (y - z) + 128.0f) / 255.0f) / #scale;
                    }
                }
                .to_string(),
            );
        } else {
            self.shader.push_str(
                &quote_declaration! {
                    fn #f(t: f32) -> f32 {
                        if t > 6.0f / 29.0f { return t * t * t; }
                        return (t - 16.0f / 116.0f) * 108.0f / 841.0f;
                    }
                }
                .to_string(),
            );
            self.shader.push_str(
                &quote_declaration! {
                    fn #ident(c: vec3f) -> vec3f {
                        let lab = c * #scale;
                        let y = (lab.x * 100.0f + 16.0f) / 116.0f;
                        let x = y + (lab.y * 255.0f - 128.0f) / 500.0f;
                        let z = y - (lab.z * 255.0f - 128.0f) / 200.0f;
                        return vec3f(#f(x), #f(y), #f(z)) * vec3f(#wx, #wy, #wz);
                    }
                }
                .to_string(),
            );
        }
        Ok(ident)
    }
}

fn samples(lut: &LutStore) -> Vec<f32> {
    match lut {
        LutStore::Store8(values) => values.iter().map(|&v| f32::from(v) / 255.0).collect(),
        LutStore::Store16(values) => values.iter().map(|&v| f32::from(v) / 65535.0).collect(),
    }
}
