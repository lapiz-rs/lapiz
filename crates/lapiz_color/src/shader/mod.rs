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
