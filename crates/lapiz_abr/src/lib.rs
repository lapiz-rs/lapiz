// DISCLAIMER
//
// This crate was developed exclusively through manual clean room reverse
// engineering, and the following statements are made with respect to that
// work:
//
// 1. The implementation relies solely on publicly available documentation of
//    the Adobe Brush (ABR) file format and on other existing, independently
//    developed implementations of that format.
//
// 2. Adobe Photoshop was used only as a reference for artifacts produced by
//    hand: sample ABR files were created manually through the Photoshop user
//    interface and examined as reference material for this crate.
//
// 3. No Adobe Photoshop binary, library, or other executable component was
//    disassembled, decompiled, or otherwise inspected.
//
// 4. No script, tool, or automated process was used to run, probe, instrument,
//    or debug Adobe Photoshop.
//
// This crate is an independent implementation of the ABR format. It contains
// no Adobe software and is not affiliated with, endorsed by, or sponsored by
// Adobe Inc.

use anyhow::{Result, bail};
pub use cursor::Cursor;
use descriptor::BrushDescriptorRoot;
pub use descriptor::{
    AbrClass, AbrEnum, AbrIntegerEnum, AbrObject, AbrValue, BlendMode, BrushGroup, BrushTip,
    ComputedBrushTip, DBrushTip, Descriptor, DescriptorUnit, DualBrush, DynamicsControl,
    EraserToolOptions, PaintToolOptions, PatternReference, PropertyDynamics, RgbColor,
    SampledBrushTip, ShToolOptions, SmudgeToolOptions, ToolOptions, UnitFloat,
};
pub use header::AbrHeader;
pub use hierarchy::HierarchyNode;
pub use lapiz_abr_derive::{AbrClass, AbrEnum, AbrIntegerEnum, AbrObject};
pub use pattern::{ColorMode, Pattern, PatternChannel};
pub use sample::{Sample, SampleImage};

mod cursor;
mod descriptor;
mod header;
mod hierarchy;
mod pattern;
mod rle;
mod sample;

#[derive(Debug)]
pub struct Abr {
    pub header: AbrHeader,
    pub descriptors: Vec<Descriptor>,
    pub hierarchy: HierarchyNode,
    pub samples: Vec<Sample>,
    pub patterns: Vec<Pattern>,
}

impl Abr {
    pub fn parse(input: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(input);
        let header = AbrHeader::parse(&mut cursor)?;
        let mut brushes = Vec::new();
        let mut hierarchy_entries = Vec::new();
        let mut samples = Vec::new();
        let mut patterns = Vec::new();

        while cursor.remaining() != 0 {
            let signature = cursor.take(4)?;
            if signature != b"8BIM" {
                bail!("unsupported ABR section signature {signature:?}");
            }

            let key = cursor.take(4)?;
            let len = usize::try_from(cursor.read_u32_be()?)?;
            let mut section = cursor.take_cursor(len)?;

            if key == b"desc" {
                brushes.extend(BrushDescriptorRoot::parse_desc_section(&mut section)?);
            } else if key == b"phry" {
                hierarchy_entries.extend(hierarchy::parse_phry_section(&mut section)?);
            } else if key == b"samp" {
                samples.extend(Sample::parse_samp_section(&mut section)?);
            } else if key == b"patt" {
                patterns.extend(Pattern::parse_patt_section(&mut section)?);
            }

            if cursor.remaining() != 0 {
                cursor.align_to(4)?;
            }
        }

        let hierarchy = HierarchyNode::from_entries(hierarchy_entries, brushes.len())?;

        Ok(Self {
            header,
            descriptors: brushes,
            hierarchy,
            samples,
            patterns,
        })
    }
}

#[doc(hidden)]
pub mod __private {
    pub use anyhow::{Error, Result};
}
