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
use cursor::Cursor;
use descriptor::class::{BrushDescriptorRoot, Descriptor};
use header::AbrHeader;
use hierarchy::HierarchyNode;
use pattern::Pattern;
use sample::Sample;

pub mod cursor;
pub mod descriptor;
pub mod header;
pub mod hierarchy;
pub mod pattern;
mod rle;
pub mod sample;

/// A parsed Adobe Brush (`.abr`) archive.
///
/// Aggregates the version header and all top-level `8BIM` sections decoded from a version 6.2+
/// ABR binary stream.
///
/// # Sections
///
/// - **Descriptors (`desc`)**: Brush preset definitions stored in [`descriptors`](Self::descriptors),
///   describing tip geometry, dynamics, scattering, texture references, dual brushes, and tool options.
/// - **Hierarchy (`phry`)**: Folder and preset organization tree stored in [`hierarchy`](Self::hierarchy),
///   mapping group nodes to indices in [`descriptors`](Self::descriptors).
/// - **Samples (`samp`)**: Grayscale alpha mask bitmaps stored in [`samples`](Self::samples),
///   referenced by [`Uuid`](uuid::Uuid) from sampled brush tips.
/// - **Patterns (`patt`)**: Multi-channel texture patterns stored in [`patterns`](Self::patterns),
///   referenced by [`Uuid`](uuid::Uuid) from textured brush presets.
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
    #![expect(
        clippy::pub_use,
        reason = "Abr derive expansions resolve these types through lapiz_abr::__private"
    )]

    pub use anyhow::{Error, Result};
}
