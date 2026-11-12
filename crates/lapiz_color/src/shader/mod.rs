use std::num::NonZeroU64;

use anyhow::Result;
use moxcms::{ColorProfile, TransformOptions};
use wgpu::{
    BindGroupEntry, BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, Device,
    ShaderStages,
};

mod pipeline;
#[cfg(test)]
mod tests;

const TABLES_GROUP_CONST: &str = "_TABLES_GROUP";
const TABLES_START_BINDING_CONST: &str = "_TABLES_START_BINDING";

/// A compiled WGSL shader fragment and GPU lookup tables for ICC color profile transformations.
///
/// `IccTransformShader` translates [`ColorProfile`] definitions—including parametric curves,
/// sampled tone reproduction curves (TRCs), CICP transfer functions, matrix-shaper transforms,
/// and 1D/3D LUT pipelines—into executable WGSL code backed by read-only storage buffers.
///
/// # Usage
///
/// 1. Construct a transform between two profiles with [`IccTransformShader::new`], or an identity
///    pass-through with [`IccTransformShader::unmanaged`].
/// 2. Call [`IccTransformShader::compile`] to obtain the WGSL source string (with bind group and
///    binding constants injected) and the corresponding [`BindGroupLayoutEntry`] list.
/// 3. Bind the storage buffers at render/compute time using [`IccTransformShader::bindings`].
pub struct IccTransformShader {
    shader: String,
    table_buffers: Vec<Buffer>,
}

impl IccTransformShader {
    pub fn new(
        ident: &str,
        src_pr: &ColorProfile,
        dst_pr: &ColorProfile,
        options: TransformOptions,
        device: &Device,
    ) -> Result<Self> {
        pipeline::build(ident, Some(src_pr), Some(dst_pr), options, device)
    }

    pub fn unmanaged(ident: &str, device: &Device) -> Result<Self> {
        pipeline::build(ident, None, None, TransformOptions::default(), device)
    }

    pub fn compile(
        &self,
        group: u32,
        start_binding: u32,
        visibility: ShaderStages,
    ) -> (String, Vec<BindGroupLayoutEntry>) {
        let shader = format!(
            "{}\nconst {}: u32 = {};\nconst {}: u32 = {};\n",
            self.shader, TABLES_GROUP_CONST, group, TABLES_START_BINDING_CONST, start_binding
        );

        let layout = self
            .table_buffers
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
        self.table_buffers
            .iter()
            .enumerate()
            .map(|(i, storage)| BindGroupEntry {
                binding: start_binding + i as u32,
                resource: storage.as_entire_binding(),
            })
            .collect()
    }
}

/// A GPU shader transform that converts colors from an input [`ColorProfile`] into CIE [`Xyz`](crate::model::xyz::Xyz) PCS.
///
/// Wraps an [`IccTransformShader`] configured with a source profile and no destination profile,
/// producing linear D50 XYZ tristimulus values in the Profile Connection Space.
pub struct IccInputTransformShader {
    transform: IccTransformShader,
}

impl IccInputTransformShader {
    pub fn new(ident: &str, profile: &ColorProfile, device: &Device) -> Result<Self> {
        Ok(Self {
            transform: pipeline::build(
                ident,
                Some(profile),
                None,
                TransformOptions {
                    allow_use_cicp_transfer: false,
                    ..Default::default()
                },
                device,
            )?,
        })
    }

    pub fn compile(
        &self,
        group: u32,
        start_binding: u32,
        visibility: ShaderStages,
    ) -> (String, Vec<BindGroupLayoutEntry>) {
        self.transform.compile(group, start_binding, visibility)
    }

    pub fn bindings(&self, start_binding: u32) -> Vec<BindGroupEntry<'_>> {
        self.transform.bindings(start_binding)
    }
}

/// A GPU shader transform that converts colors from CIE [`Xyz`](crate::model::xyz::Xyz) PCS into an output [`ColorProfile`].
///
/// Wraps an [`IccTransformShader`] configured with no source profile and a destination profile,
/// mapping linear D50 XYZ values from the Profile Connection Space into the target device or working space.
pub struct IccOutputTransformShader {
    transform: IccTransformShader,
}

impl IccOutputTransformShader {
    pub fn new(ident: &str, profile: &ColorProfile, device: &Device) -> Result<Self> {
        Ok(Self {
            transform: pipeline::build(
                ident,
                None,
                Some(profile),
                TransformOptions {
                    allow_use_cicp_transfer: false,
                    ..Default::default()
                },
                device,
            )?,
        })
    }

    pub fn compile(
        &self,
        group: u32,
        start_binding: u32,
        visibility: ShaderStages,
    ) -> (String, Vec<BindGroupLayoutEntry>) {
        self.transform.compile(group, start_binding, visibility)
    }

    pub fn bindings(&self, start_binding: u32) -> Vec<BindGroupEntry<'_>> {
        self.transform.bindings(start_binding)
    }
}
