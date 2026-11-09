#[cfg(test)]
mod tests;

use std::num::NonZeroU64;

use anyhow::{Result, ensure};
use moxcms::{
    CmsError, ColorProfile, DataColorSpace, Matrix3d, ParametricCurve, ToneReprCurve,
    TransformOptions,
};
use wesl::syntax::{
    AccessMode, AddressSpace, Attribute, BinaryExpression, BinaryOperator, CompoundStatement,
    Declaration, DeclarationKind, ElseClause, Expression, FormalParameter, Function, FunctionCall,
    GlobalDeclaration, Ident, IfClause, IfStatement, IndexingExpression, LiteralExpression,
    NamedComponentExpression, ReturnStatement, Span, Spanned, Statement, TemplateArg,
    TypeExpression,
};
use wesl_quote::quote_declaration;
use wgpu::{
    BindGroupEntry, BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferUsages,
    Device, ShaderStages,
    util::{BufferInitDescriptor, DeviceExt as _},
};

const TRC_TABLES_GROUP_CONST: &str = "_TRC_TABLES_GROUP";
const TRC_TABLES_START_BINDING_CONST: &str = "_TRC_TABLES_START_BINDING";

pub struct IccTransformShader {
    shader: String,
    trc_storages: Vec<Buffer>,
}

impl IccTransformShader {
    pub fn new(
        ident: &str,
        src_pr: &ColorProfile,
        dst_pr: &ColorProfile,
        _: TransformOptions,
        device: &Device,
    ) -> Result<Self> {
        let src_trcs = profile_trcs(src_pr)?;
        let dst_trcs = profile_trcs(dst_pr)?;
        let mut shader = String::new();
        let mut trc_storages = Vec::new();

        let [sr, sg, sb, dr, dg, db] = ["src_r", "src_g", "src_b", "dst_r", "dst_g", "dst_b"]
            .map(|channel| Ident::new(format!("_{ident}_{channel}_trc")));
        for ((name, trc), inverse) in [&sr, &sg, &sb]
            .into_iter()
            .zip(src_trcs)
            .map(|pair| (pair, false))
            .chain(
                [&dr, &dg, &db]
                    .into_iter()
                    .zip(dst_trcs)
                    .map(|pair| (pair, true)),
            )
        {
            let (func, trc_storage) = trc_function(name, trc, inverse)?;
            shader.push_str(&func.to_string());

            if let Some(storage) = trc_storage {
                let storage_ident = &storage.table;
                let storage_len = storage.samples.len() as u32;
                let storage_index = trc_storages.len() as u32;
                let group = Ident::new(TRC_TABLES_GROUP_CONST.to_owned());
                let start_binding = Ident::new(TRC_TABLES_START_BINDING_CONST.to_owned());
                shader.push_str(
                    &quote_declaration! {
                        @group(#group) @binding(#start_binding + #storage_index) var<storage, read> #storage_ident: array<f32, #storage_len>;
                    }
                    .to_string(),
                );
                trc_storages.push(storage);
            }
        }

        let matrix = Ident::new(format!("_{ident}_combined_matrix"));
        shader.push_str(&matrix_function(&matrix, &src_pr.transform_matrix(dst_pr)).to_string());
        shader.push_str(
            &quote_declaration! {
                fn #ident(c: vec3f) -> vec3f {
                    let rgb = #matrix(vec3f(#sr(c.x), #sg(c.y), #sb(c.z)));
                    return vec3f(#dr(rgb.x), #dg(rgb.y), #db(rgb.z));
                }
            }
            .to_string(),
        );

        let trc_storages = trc_storages
            .into_iter()
            .map(|storage| {
                device.create_buffer_init(&BufferInitDescriptor {
                    label: Some(&storage.table.to_string()),
                    contents: bytemuck::cast_slice(&storage.samples),
                    usage: BufferUsages::STORAGE,
                })
            })
            .collect();

        Ok(Self {
            shader,
            trc_storages,
        })
    }

    pub fn unmanaged(ident: &str) -> Self {
        Self {
            shader: quote_declaration! {
                fn #ident(c: vec3f) -> vec3f { return c; }
            }
            .to_string(),
            trc_storages: Vec::new(),
        }
    }

    pub fn compile(
        &self,
        group: u32,
        start_binding: u32,
        visibility: ShaderStages,
    ) -> (String, Vec<BindGroupLayoutEntry>) {
        let shader = format!(
            "{}\nconst {}: u32 = {};\nconst {}: u32 = {};\n",
            self.shader,
            TRC_TABLES_GROUP_CONST,
            group,
            TRC_TABLES_START_BINDING_CONST,
            start_binding
        );

        let layout = self
            .trc_storages
            .iter()
            .enumerate()
            .map(|(i, storage)| BindGroupLayoutEntry {
                binding: start_binding + i as u32,
                visibility,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: Some(NonZeroU64::new(storage.size()).unwrap()),
                },
                count: None,
            })
            .collect();

        (shader, layout)
    }

    pub fn bindings(&self, start_binding: u32) -> Vec<BindGroupEntry<'_>> {
        self.trc_storages
            .iter()
            .enumerate()
            .map(|(i, storage)| BindGroupEntry {
                binding: start_binding + i as u32,
                resource: storage.as_entire_binding(),
            })
            .collect()
    }
}

pub struct ToneReprCurveStorage {
    table: Ident,
    samples: Vec<f32>,
}

fn profile_trcs(profile: &ColorProfile) -> Result<[&ToneReprCurve; 3]> {
    ensure!(
        profile.color_space == DataColorSpace::Rgb,
        "RGB color space required"
    );
    ensure!(profile.is_matrix_shaper(), "Matrix shaper profile required");
    Ok([
        profile
            .red_trc
            .as_ref()
            .ok_or(CmsError::BuildTransferFunction)?,
        profile
            .green_trc
            .as_ref()
            .ok_or(CmsError::BuildTransferFunction)?,
        profile
            .blue_trc
            .as_ref()
            .ok_or(CmsError::BuildTransferFunction)?,
    ])
}

fn trc_function(
    ident: &Ident,
    trc: &ToneReprCurve,
    inverse: bool,
) -> Result<(GlobalDeclaration, Option<ToneReprCurveStorage>)> {
    match trc {
        ToneReprCurve::Lut(lut) if lut.is_empty() => Ok((
            quote_declaration! {
                fn #ident(x: f32) -> f32 {
                    return x;
                }
            },
            None,
        )),
        ToneReprCurve::Lut(lut) if lut.len() == 1 => {
            let gamma = f32::from(lut[0]) / 256.0;
            ensure!(gamma > 0.0, "Positive gamma required");
            let gamma = if inverse { gamma.recip() } else { gamma };
            Ok((
                quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        return pow(clamp(x, 0.0f, 1.0f), #gamma);
                    }
                },
                None,
            ))
        }
        ToneReprCurve::Lut(lut) => {
            let samples = if inverse {
                const INVERSE_SAMPLES: u32 = 16384;
                let evaluator = trc.make_gamma_evaluator()?;
                (0..INVERSE_SAMPLES)
                    .map(|i| evaluator.evaluate_value(i as f32 / (INVERSE_SAMPLES - 1) as f32))
                    .collect::<Vec<_>>()
            } else {
                lut.iter()
                    .map(|&value| f32::from(value) / 65535.0)
                    .collect()
            };
            let last = samples.len() as u32 - 1;
            let scale = last as f32;

            let table = Ident::new(format!("{ident}_trc_table"));

            Ok((
                quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        let position = clamp(x, 0.0f, 1.0f) * #scale;
                        let lower = u32(position);
                        let upper = min(lower + 1u, #last);
                        return mix(#table[lower], #table[upper], position - f32(lower));
                    }
                },
                Some(ToneReprCurveStorage { table, samples }),
            ))
        }
        ToneReprCurve::Parametric(parameters) => {
            ensure!(
                matches!(parameters.len(), 1 | 3 | 4 | 5 | 7)
                    && parameters.iter().all(|value| value.is_finite()),
                "Valid finite parametric curve parameters required"
            );
            let curve = ParametricCurve::new(parameters).ok_or(CmsError::BuildTransferFunction)?;
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

            Ok((
                quote_declaration! {
                    fn #ident(x: f32) -> f32 {
                        if x < #d {
                            return #c * x + #f;
                        } else {
                            return pow(#a * x + #b, #g) + #e;
                        }
                    }
                },
                None,
            ))
        }
    }
}

fn matrix_function(ident: &Ident, matrix: &Matrix3d) -> GlobalDeclaration {
    let m = matrix.v.map(|row| row.map(|value| value as f32));
    let [a, b, c] = m[0];
    let [d, e, f] = m[1];
    let [g, h, i] = m[2];
    quote_declaration! {
        fn #ident(x: vec3f) -> vec3f {
            // WGSL matrices use columns, moxcms matrices use rows.
            return mat3x3f(#a, #d, #g, #b, #e, #h, #c, #f, #i) * x;
        }
    }
}
